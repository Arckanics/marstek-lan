//! Dérivation de l'identifiant de topic cloud d'un appareil Marstek à partir du couple `salt`
//! fourni par `getDeviceList`, de sa MAC et de son type (vid). Port de l'algorithme `cq` de
//! l'app (SHA-256 + permutation façon RC4 + flux LCG/XOR + encodage base62), vérifié octet pour
//! octet contre un vecteur réel (voir les tests).

use sha2::{Digest, Sha256};

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Flux LCG dérivé de `key`, XORé sur les octets UTF-8 de `data`, rendu en hexadécimal.
fn stream_e(data: &str, key: &str) -> String {
    let len = data.chars().count().max(1);
    let mut seed: i64 = 0;
    for b in key.bytes() {
        seed = (seed * 31 + i64::from(b)) % 2_147_483_647;
    }
    let mut state = seed as u64;
    let keystream: Vec<u8> = (0..len)
        .map(|_| {
            state = (state * 1_664_525 + 1_013_904_223) % 4_294_967_296;
            ((state ^ (state >> 16)) & 0xff) as u8
        })
        .collect();
    let out: Vec<u8> = data.bytes().enumerate().map(|(i, b)| b ^ keystream[i % keystream.len()]).collect();
    to_hex(&out)
}

/// SHA-256 de `input`, réordonné en 24 octets, puis deux caractères base62 par octet.
fn code_e(input: &str) -> String {
    let hash = Sha256::digest(input.as_bytes());
    let words: [u32; 8] = std::array::from_fn(|i| u32::from_be_bytes([hash[i * 4], hash[i * 4 + 1], hash[i * 4 + 2], hash[i * 4 + 3]]));
    const CHARSET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut out = String::with_capacity(48);
    for i in 0..24u32 {
        let byte = ((words[(i % 8) as usize] >> ((i / 8) * 8)) & 0xff) as usize;
        out.push(CHARSET[byte % 62] as char);
        out.push(CHARSET[(byte * 31) % 62] as char);
    }
    out
}

/// Table de permutation façon KSA de RC4, graine = `key`.
fn permutation(key: &[u8], size: usize) -> Vec<usize> {
    let mut p: Vec<usize> = (0..size).collect();
    if size <= 1 {
        return p;
    }
    let mut j = 0usize;
    for i in 0..size {
        j = (j + p[i] + usize::from(key[i % key.len()])) % size;
        p.swap(i, j);
    }
    p
}

/// Brouille les octets UTF-8 de `content` par la permutation dérivée de `key`, N fois (N tiré du
/// dernier octet de `content`), rendu en hexadécimal.
fn text_for_rand(content: &str, key: &str) -> String {
    if content.is_empty() || key.is_empty() {
        return String::new();
    }
    let mut data: Vec<u8> = content.bytes().collect();
    let n = (*data.last().unwrap() % 5).max(1);
    let key_bytes: Vec<u8> = key.bytes().collect();
    for _ in 0..n {
        let p = permutation(&key_bytes, data.len());
        data = p.iter().map(|&idx| data[idx]).collect();
    }
    to_hex(&data)
}

/// L'identifiant de topic cloud (`cq`) d'un appareil, ou `None` si la MAC est trop courte.
fn cq(salt: &str, mac: &str, vid: &str) -> Option<String> {
    let len = mac.chars().count();
    if len < 4 {
        return None;
    }
    let var1 = format!("{vid}_{}", &mac[..len - 4]);
    let var2 = format!("{}_{vid}", &mac[1..len - 2]);
    let h1 = text_for_rand(salt, &var1);
    let h2 = stream_e(&format!("{vid}{mac}"), &var2);
    Some(code_e(&format!("{h1}{h2}")).chars().take(24).collect())
}

/// L'identifiant de topic à partir du couple `salt` (`"<courant>,<émis>"`) renvoyé par le cloud.
///
/// `None` quand aucun identifiant n'est en service : `"0"` en tête (émis mais pas encore pris par
/// l'appareil, posé seulement en Bluetooth), moitié vide, ou couple malformé. L'app retombe alors
/// sur l'identifiant par chiffrement AES du topic.
// shortcut: repli AES non géré (seuls les appareils dont le salt donne un id "confirmed"/"rotating"
// sont pris en charge) ; à ajouter si un appareil renvoie un couple pending/unusable.
pub fn resolve_topic_id(salt_pair: &str, mac: &str, vid: &str) -> Option<String> {
    let (current, issued) = salt_pair.split_once(',')?;
    if current == "0" || current.is_empty() || issued.is_empty() || issued == "0" {
        return None;
    }
    let base = if issued == current { issued } else { current };
    cq(base, mac, vid).filter(|id| !id.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vecteurs produits par l'implémentation de référence (topic.ts de hame-relay). Le premier est
    /// réel : salt capté du compte, et l'id qui en sort est celui utilisé avec succès contre la
    /// Venus D (firmware 147).
    #[test]
    fn cq_matches_reference_vectors() {
        let cases = [
            ("84905ed2bc7ab1e5", "bc2a3314cc36", "VNSD-0", "XVC0fVI0TV3VJVm05Vk020i0"),
            ("0000000000000000", "bc2a3314cc36", "VNSD-0", "9V20S0s0RVe08080A0U0C0HV"),
            ("deadbeefcafe1234", "aabbccddeeff", "VNSD-0", "zVw0jVvVw0q0vVRVO0jVtVg0"),
            ("1a2b3c4d5e6f7080", "112233445566", "HMG-1", "Q0k0XV7V40G0Q0NVe0TVBVTV"),
            ("84905ed2bc7ab1e5", "bc2a3314cc36", "VNSD-1", "y0xV005VI0c0BV5VfVm0y0JV"),
        ];
        for (salt, mac, vid, want) in cases {
            assert_eq!(cq(salt, mac, vid).as_deref(), Some(want), "cq({salt},{mac},{vid})");
        }
    }

    #[test]
    fn cq_rejects_short_mac() {
        assert_eq!(cq("abcd", "ab", "VNSD-0"), None);
    }

    #[test]
    fn resolve_uses_issued_when_confirmed_and_real_device_matches() {
        assert_eq!(
            resolve_topic_id("84905ed2bc7ab1e5,84905ed2bc7ab1e5", "bc2a3314cc36", "VNSD-0").as_deref(),
            Some("XVC0fVI0TV3VJVm05Vk020i0"),
        );
    }

    #[test]
    fn resolve_uses_current_when_rotating() {
        // courant != émis : on adresse l'appareil par l'id courant.
        let rotating = resolve_topic_id("84905ed2bc7ab1e5,ffffffffffffffff", "bc2a3314cc36", "VNSD-0");
        assert_eq!(rotating.as_deref(), Some("XVC0fVI0TV3VJVm05Vk020i0"));
    }

    #[test]
    fn resolve_rejects_pending_or_malformed() {
        assert_eq!(resolve_topic_id("0,84905ed2bc7ab1e5", "bc2a3314cc36", "VNSD-0"), None);
        assert_eq!(resolve_topic_id("84905ed2bc7ab1e5,0", "bc2a3314cc36", "VNSD-0"), None);
        assert_eq!(resolve_topic_id(",", "bc2a3314cc36", "VNSD-0"), None);
        assert_eq!(resolve_topic_id("nocomma", "bc2a3314cc36", "VNSD-0"), None);
    }
}
