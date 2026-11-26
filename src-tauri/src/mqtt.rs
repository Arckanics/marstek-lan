//! Publication d'un unique message MQTT vers le broker cloud Marstek (AWS IoT), pour rouvrir l'Open API
//! locale d'un appareil quand la seule voie restante est le cloud (API locale coupée après un redémarrage).
//!
//! Transport maison : CONNECT → PUBLISH QoS 1 → PUBACK → DISCONNECT, sur une socket TLS mutuelle (rustls).
//! Le certificat client de la passerelle hame est embarqué (`certs/`, hors dépôt — voir `vendor-certs.ps1`).
//! Aucun secret ni détail d'erreur n'est journalisé : tout échec devient `Msg::CloudMqtt`.

use crate::i18n::Msg;
use rustls::pki_types::{CertificateDer, ServerName};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

// Certs embarqués (CARGO_MANIFEST_DIR = src-tauri). Absents du dépôt : `vendor-certs.ps1` les pose, `build.rs` les vérifie.
const CA: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/certs/ca.crt"));
const CLIENT_CERT: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/certs/hame-2025.crt"));
const CLIENT_KEY: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/certs/hame-2025.key"));

const HOST: &str = "a40nr6osvmmaw-ats.iot.eu-west-3.amazonaws.com";
const PORT: u16 = 8883;
const IO_TIMEOUT: Duration = Duration::from_secs(10);
/// Plafond de la longueur restante d'un paquet lu : nos réponses attendues (CONNACK, PUBACK) font 2 octets. Borne pour
/// qu'un broker hostile ou un intermédiaire ne puisse pas nous faire allouer un tampon géant (déni de service).
const MAX_PACKET: usize = 4096;

/// Longueur restante MQTT (varint 7 bits, bit de poids fort = continuation).
fn encode_len(mut n: usize, out: &mut Vec<u8>) {
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n > 0 {
            b |= 0x80;
        }
        out.push(b);
        if n == 0 {
            break;
        }
    }
}

/// CONNECT : MQTT 3.1.1, session propre, sans will ni identifiants (l'auth est le certificat client), keep-alive 30 s.
fn connect_packet(client_id: &str) -> Vec<u8> {
    let mut body = vec![0x00, 0x04, b'M', b'Q', b'T', b'T', 0x04, 0x02, 0x00, 0x1e];
    let cid = client_id.as_bytes();
    body.extend_from_slice(&(cid.len() as u16).to_be_bytes());
    body.extend_from_slice(cid);
    let mut pkt = vec![0x10];
    encode_len(body.len(), &mut pkt);
    pkt.extend_from_slice(&body);
    pkt
}

/// PUBLISH QoS 1 (fanions `0x32`), avec identifiant de paquet pour attendre le PUBACK.
fn publish_packet(topic: &str, packet_id: u16, payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    body.extend_from_slice(topic.as_bytes());
    body.extend_from_slice(&packet_id.to_be_bytes());
    body.extend_from_slice(payload);
    let mut pkt = vec![0x32];
    encode_len(body.len(), &mut pkt);
    pkt.extend_from_slice(&body);
    pkt
}

/// SUBSCRIBE (fanions `0x02` imposés) à un seul topic en QoS 0, avec identifiant de paquet pour attendre le SUBACK.
fn subscribe_packet(topic: &str, packet_id: u16) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&packet_id.to_be_bytes());
    body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    body.extend_from_slice(topic.as_bytes());
    body.push(0x00); // QoS 0 demandé
    let mut pkt = vec![0x82];
    encode_len(body.len(), &mut pkt);
    pkt.extend_from_slice(&body);
    pkt
}

fn read_byte(r: &mut impl Read) -> std::io::Result<u8> {
    let mut b = [0u8; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}

/// Lit un paquet : octet d'en-tête (type + fanions) puis le corps de la longueur restante annoncée.
fn read_packet(r: &mut impl Read) -> std::io::Result<(u8, Vec<u8>)> {
    let header = read_byte(r)?;
    let (mut len, mut mult) = (0usize, 1usize);
    loop {
        let b = read_byte(r)?;
        len += (b & 0x7f) as usize * mult;
        if b & 0x80 == 0 {
            break;
        }
        mult *= 128;
        if mult > 128 * 128 * 128 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "longueur restante invalide"));
        }
    }
    if len > MAX_PACKET {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "paquet trop grand"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok((header, body))
}

fn tls_config() -> Result<ClientConfig, Box<dyn std::error::Error>> {
    let mut roots = RootCertStore::empty();
    for c in rustls_pemfile::certs(&mut &CA[..]) {
        roots.add(c?)?;
    }
    let client_certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut &CLIENT_CERT[..]).collect::<Result<_, _>>()?;
    let key = rustls_pemfile::private_key(&mut &CLIENT_KEY[..])?.ok_or("clé client absente")?;
    Ok(ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_client_auth_cert(client_certs, key)?)
}

/// Établit la session : TLS mutuel, CONNECT, attente d'un CONNACK accepté. Prête à publier ou s'abonner.
fn connect(client_id: &str) -> Result<StreamOwned<ClientConnection, TcpStream>, Box<dyn std::error::Error>> {
    let config = tls_config()?;
    let server = ServerName::try_from(HOST)?.to_owned();
    let addr = (HOST, PORT).to_socket_addrs()?.next().ok_or("hôte introuvable")?;
    let tcp = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))?;
    tcp.set_write_timeout(Some(IO_TIMEOUT))?;
    let conn = ClientConnection::new(Arc::new(config), server)?;
    let mut tls = StreamOwned::new(conn, tcp);
    tls.write_all(&connect_packet(client_id))?;
    tls.flush()?;
    let (header, body) = read_packet(&mut tls)?;
    if header != 0x20 || body.len() < 2 || body[1] != 0x00 {
        return Err("CONNACK refusé".into());
    }
    Ok(tls)
}

/// Publie `payload` sur `topic` du broker cloud, puis se déconnecte. `Msg::CloudMqtt` en cas d'échec, sans détail.
pub fn publish(client_id: &str, topic: &str, payload: &[u8]) -> Result<(), Msg> {
    publish_inner(client_id, topic, payload).map_err(|_| Msg::CloudMqtt)
}

fn publish_inner(client_id: &str, topic: &str, payload: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let mut tls = connect(client_id)?;
    tls.write_all(&publish_packet(topic, 1, payload))?;
    tls.flush()?;
    let (header, body) = read_packet(&mut tls)?;
    if header != 0x40 || body != 1u16.to_be_bytes() {
        return Err("PUBACK manquant".into());
    }
    let _ = tls.write_all(&[0xe0, 0x00]); // DISCONNECT, au mieux.
    Ok(())
}

/// Publie `payload` sur `pub_topic`, écoute la réponse de l'appareil sur `sub_topic` jusqu'à `wait`, puis se déconnecte.
/// Rend le texte de la réponse, ou `None` si l'appareil n'a rien renvoyé à temps. `Msg::CloudMqtt` en cas d'échec.
pub fn request(client_id: &str, sub_topic: &str, pub_topic: &str, payload: &[u8], wait: Duration) -> Result<Option<String>, Msg> {
    request_inner(client_id, sub_topic, pub_topic, payload, wait).map_err(|_| Msg::CloudMqtt)
}

fn request_inner(client_id: &str, sub_topic: &str, pub_topic: &str, payload: &[u8], wait: Duration) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let mut tls = connect(client_id)?;

    tls.write_all(&subscribe_packet(sub_topic, 1))?;
    tls.flush()?;
    let (header, body) = read_packet(&mut tls)?;
    if header != 0x90 || body.len() < 3 || body[2] & 0x80 != 0 {
        return Err("SUBACK refusé".into());
    }

    tls.write_all(&publish_packet(pub_topic, 2, payload))?;
    tls.flush()?;
    let (header, body) = read_packet(&mut tls)?;
    if header != 0x40 || body != 2u16.to_be_bytes() {
        return Err("PUBACK manquant".into());
    }

    // Lit les paquets entrants jusqu'à la réponse de l'appareil ou l'échéance. On ne PUBACK pas une réponse QoS 1 :
    // on se déconnecte juste après (session propre).
    let deadline = std::time::Instant::now() + wait;
    let mut reply = None;
    while std::time::Instant::now() < deadline {
        let left = deadline.saturating_duration_since(std::time::Instant::now()).max(Duration::from_millis(1));
        tls.sock.set_read_timeout(Some(left))?;
        match read_packet(&mut tls) {
            Ok((header, body)) if header & 0xf0 == 0x30 => {
                if let Some(text) = publish_payload(header, &body, sub_topic) {
                    reply = Some(text);
                    break;
                }
            }
            Ok(_) => {}
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break,
            Err(e) => return Err(e.into()),
        }
    }
    let _ = tls.write_all(&[0xe0, 0x00]);
    Ok(reply)
}

/// Extrait le payload d'un PUBLISH entrant si son topic est `want`. Saute l'identifiant de paquet pour QoS > 0.
fn publish_payload(header: u8, body: &[u8], want: &str) -> Option<String> {
    let tlen = u16::from_be_bytes([*body.first()?, *body.get(1)?]) as usize;
    let mut off = 2 + tlen;
    if body.get(2..off)? != want.as_bytes() {
        return None;
    }
    if (header >> 1) & 0x03 > 0 {
        off += 2;
    }
    Some(String::from_utf8_lossy(body.get(off..)?).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn remaining_length_uses_the_7bit_varint() {
        let enc = |n| {
            let mut v = Vec::new();
            encode_len(n, &mut v);
            v
        };
        assert_eq!(enc(0), [0x00]);
        assert_eq!(enc(127), [0x7f]);
        assert_eq!(enc(128), [0x80, 0x01]);
        assert_eq!(enc(16_383), [0xff, 0x7f]);
        assert_eq!(enc(16_384), [0x80, 0x80, 0x01]);
    }

    #[test]
    fn connect_packet_is_a_clean_mqtt_311_session() {
        let pkt = connect_packet("mst_abc");
        // En-tête CONNECT + longueur restante (10 var. header + 2 + 7 payload = 19).
        assert_eq!(&pkt[..8], &[0x10, 19, 0x00, 0x04, b'M', b'Q', b'T', b'T']);
        assert_eq!(pkt[8], 0x04); // niveau 3.1.1
        assert_eq!(pkt[9], 0x02); // fanion session propre, sans will ni identifiants
        assert_eq!(&pkt[10..12], &[0x00, 0x1e]); // keep-alive 30 s
        assert_eq!(&pkt[12..14], &[0x00, 0x07]); // longueur du client id
        assert_eq!(&pkt[14..], b"mst_abc");
    }

    #[test]
    fn publish_packet_carries_topic_packet_id_and_payload() {
        let pkt = publish_packet("a/b", 1, b"cd=30,api=1,port=30000");
        assert_eq!(pkt[0], 0x32); // PUBLISH QoS 1
        let (header, body) = read_packet(&mut Cursor::new(pkt)).unwrap();
        assert_eq!(header, 0x32);
        assert_eq!(&body[..2], &[0x00, 0x03]); // longueur du topic
        assert_eq!(&body[2..5], b"a/b");
        assert_eq!(&body[5..7], &[0x00, 0x01]); // id de paquet
        assert_eq!(&body[7..], b"cd=30,api=1,port=30000");
    }

    #[test]
    fn subscribe_packet_asks_for_one_topic_at_qos0() {
        let pkt = subscribe_packet("a/b", 1);
        assert_eq!(pkt[0], 0x82); // SUBSCRIBE, fanions imposés
        let (header, body) = read_packet(&mut Cursor::new(pkt)).unwrap();
        assert_eq!(header, 0x82);
        assert_eq!(&body[..2], &[0x00, 0x01]); // id de paquet
        assert_eq!(&body[2..4], &[0x00, 0x03]); // longueur du topic
        assert_eq!(&body[4..7], b"a/b");
        assert_eq!(body[7], 0x00); // QoS 0 demandé
    }

    #[test]
    fn publish_payload_extracts_only_the_matching_topic() {
        // PUBLISH QoS 0 : longueur topic + topic + payload (pas d'id de paquet).
        let mut q0 = vec![0x00, 0x03];
        q0.extend_from_slice(b"a/b");
        q0.extend_from_slice(b"cd=18,meter");
        assert_eq!(publish_payload(0x30, &q0, "a/b").as_deref(), Some("cd=18,meter"));
        assert_eq!(publish_payload(0x30, &q0, "x/y"), None);
        // QoS 1 : un id de paquet (2 octets) précède le payload et doit être sauté.
        let mut q1 = vec![0x00, 0x03];
        q1.extend_from_slice(b"a/b");
        q1.extend_from_slice(&[0x00, 0x07]);
        q1.extend_from_slice(b"cd=18,err_cmd");
        assert_eq!(publish_payload(0x32, &q1, "a/b").as_deref(), Some("cd=18,err_cmd"));
    }

    #[test]
    fn read_packet_rejects_an_oversized_length() {
        // Longueur annoncée ~16 Mo : refusée avant toute allocation.
        assert!(read_packet(&mut Cursor::new(vec![0x30, 0x80, 0x80, 0x80, 0x08])).is_err());
    }

    #[test]
    fn read_packet_decodes_a_multibyte_length() {
        let mut pkt = vec![0x32];
        encode_len(200, &mut pkt);
        pkt.extend(std::iter::repeat_n(0x41u8, 200));
        let (header, body) = read_packet(&mut Cursor::new(pkt)).unwrap();
        assert_eq!(header, 0x32);
        assert_eq!(body.len(), 200);
    }

    /// Les certs embarqués forment une config mTLS valide (clé RSA PKCS#1, racine acceptée).
    #[test]
    fn embedded_certificates_build_a_tls_config() {
        assert!(tls_config().is_ok());
    }

    /// Réseau + non destructif : poignée de main mTLS + CONNECT contre le vrai broker AWS IoT, sans aucune
    /// publication (l'appareil n'est pas touché). Prouve l'interop du framing maison. Lancer explicitement :
    /// `cargo test -p marstek-lan connect_handshake_against_the_real_broker -- --ignored --nocapture`.
    #[test]
    #[ignore = "réseau : contacte le broker cloud Marstek"]
    fn connect_handshake_against_the_real_broker() {
        let config = tls_config().unwrap();
        let server = ServerName::try_from(HOST).unwrap().to_owned();
        let addr = (HOST, PORT).to_socket_addrs().unwrap().next().unwrap();
        let tcp = TcpStream::connect_timeout(&addr, IO_TIMEOUT).unwrap();
        tcp.set_read_timeout(Some(IO_TIMEOUT)).unwrap();
        tcp.set_write_timeout(Some(IO_TIMEOUT)).unwrap();
        let conn = ClientConnection::new(Arc::new(config), server).unwrap();
        let mut tls = StreamOwned::new(conn, tcp);
        tls.write_all(&connect_packet("mst_0123456789abcdef01234567")).unwrap();
        tls.flush().unwrap();
        let (header, body) = read_packet(&mut tls).unwrap();
        let _ = tls.write_all(&[0xe0, 0x00]);
        assert_eq!(header, 0x20, "type CONNACK");
        assert_eq!(body[1], 0x00, "code retour CONNACK");
    }
}
