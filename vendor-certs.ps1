#!/usr/bin/env pwsh
#requires -version 7
<#
.SYNOPSIS
  Récupère le certificat client mTLS de la passerelle hame (broker « hame-2025 ») depuis l'image Docker
  publique tomquist/hame-relay et l'écrit dans src-tauri/certs/ (dossier gitignoré, jamais committé).

  Ces fichiers sont requis pour compiler : mqtt.rs les embarque (include_bytes!) et build.rs vérifie leur sha256.
  Ils ne sont pas dans le dépôt Git de hame-relay (gitignorés côté amont) ; leur seule source publique est l'image.

  L'image est épinglée par digest pour la reproductibilité ; chaque fichier est vérifié par sha256.
  Pour pointer une image plus récente : mettre à jour $Image, les sha256 ci-dessous ET ceux de src-tauri/build.rs.
#>
[CmdletBinding()]
param(
  [string]$Image = "sha256:3027a288c501197fe2dc2cc8fb52d4f16a622d38c0f005e338168db8e85b8b0d",
  [string]$Repo  = "tomquist/hame-relay"
)
$ErrorActionPreference = "Stop"

$expected = [ordered]@{
  "ca.crt"        = "2c43952ee9e000ff2acc4e2ed0897c0a72ad5fa72c3d934e81741cbd54f05bd1"
  "hame-2025.crt" = "2502b161f5f533defa924840b2c2764440eea27b12b910c5b23aeb214715984e"
  "hame-2025.key" = "6ceeedbacda597f98bc4605518c7e750101635dbf11f327d06ab215eb44886e4"
}
$dest = Join-Path $PSScriptRoot "src-tauri/certs"
New-Item -ItemType Directory -Force $dest | Out-Null

$base = "https://ghcr.io/v2/$Repo"
$accept = "application/vnd.oci.image.index.v1+json,application/vnd.docker.distribution.manifest.list.v2+json,application/vnd.oci.image.manifest.v1+json,application/vnd.docker.distribution.manifest.v2+json"
$token = (Invoke-RestMethod "https://ghcr.io/token?scope=repository:${Repo}:pull&service=ghcr.io").token
$hdr = @{ Authorization = "Bearer $token"; Accept = $accept }

$man = Invoke-RestMethod -Headers $hdr "$base/manifests/$Image"
if ($man.manifests) {
  $amd = $man.manifests | Where-Object { $_.platform.os -eq "linux" -and $_.platform.architecture -eq "amd64" } | Select-Object -First 1
  if (-not $amd) { throw "aucune image linux/amd64 dans l'index $Image" }
  $man = Invoke-RestMethod -Headers $hdr "$base/manifests/$($amd.digest)"
}

$tmp = Join-Path ([IO.Path]::GetTempPath()) ("hame-certs-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force $tmp | Out-Null
try {
  $found = @{}
  $layers = @($man.layers); [array]::Reverse($layers) # certs copiés tard dans le Dockerfile
  foreach ($l in $layers) {
    if ($found.Count -eq $expected.Count) { break }
    $tgz = Join-Path $tmp "layer.tgz"
    Invoke-WebRequest -Headers @{ Authorization = "Bearer $token" } "$base/blobs/$($l.digest)" -OutFile $tgz
    $ex = Join-Path $tmp "ex"
    Remove-Item -Recurse -Force $ex -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Force $ex | Out-Null
    & tar -xzf $tgz -C $ex 2>$null
    foreach ($name in $expected.Keys) {
      if ($found.ContainsKey($name)) { continue }
      $hit = Get-ChildItem $ex -Recurse -File -Filter $name -ErrorAction SilentlyContinue |
             Where-Object { $_.FullName -match "[\\/]certs[\\/]" } | Select-Object -First 1
      if ($hit) { $found[$name] = $hit.FullName }
    }
  }
  foreach ($name in $expected.Keys) {
    if (-not $found.ContainsKey($name)) { throw "introuvable dans l'image : $name" }
    $h = (Get-FileHash $found[$name] -Algorithm SHA256).Hash.ToLower()
    if ($h -ne $expected[$name]) { throw "$name : sha256 inattendu ($h), image corrompue ou certs tournés" }
    Copy-Item $found[$name] (Join-Path $dest $name) -Force
    "ok  $name"
  }
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}
"Certificats écrits dans $dest"
