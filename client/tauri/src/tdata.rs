// Экспорт сессии веб-клиента в формат tdata Telegram Desktop.
//
// Структура (один аккаунт, без локального код-пароля):
//   tdata/key_datas            — соль, зашифрованный локальный ключ и список аккаунтов
//   tdata/D877F783D5D3EF8Cs    — MTP-авторизация (id пользователя, основной DC, ключи по DC)
//   tdata/D877F783D5D3EF8C/maps — пустая карта файлов аккаунта
//
// Каждый файл: "TDF$" + версия (LE) + данные QDataStream (BE) + MD5.
// Шифрование — как `PrepareEncrypted` в tdesktop: SHA-1 от данных как msg_key,
// ключ и IV по схеме MTProto 1.0, AES-256-IGE.

use std::fs;
use std::path::Path;

use aes::Aes256;
use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit, generic_array::GenericArray};
use md5::{Digest as _, Md5};
use sha1::Sha1;
use sha2::Sha512;

const TDF_MAGIC: &[u8; 4] = b"TDF$";
const TDF_VERSION: i32 = 3_004_000;
const AUTH_KEY_SIZE: usize = 256;
const LOCAL_SALT_SIZE: usize = 32;
// Без код-пароля tdesktop использует одну итерацию PBKDF2
const PASSCODE_ITERATIONS: u32 = 1;
const DBI_MTP_AUTHORIZATION: u32 = 0x4b;
// Признак 64-битного id пользователя в сериализованной авторизации
const WIDE_IDS_TAG: u64 = u64::MAX;
const ACCOUNT_DATA_NAME: &str = "data";

pub struct Session {
  pub user_id: u64,
  pub main_dc_id: i32,
  pub keys: Vec<(i32, [u8; AUTH_KEY_SIZE])>,
}

/// Записывает tdata в `dir` (папка должна не существовать или быть пустой)
pub fn write(dir: &Path, session: &Session) -> Result<(), String> {
  let local_key = random_bytes::<AUTH_KEY_SIZE>()?;
  let salt = random_bytes::<LOCAL_SALT_SIZE>()?;
  let passcode_key = create_local_key(&[], &salt);

  // key_datas
  let mut key_payload = Vec::new();
  key_payload.extend_from_slice(&local_key);
  let key_encrypted = prepare_encrypted(&key_payload, &passcode_key)?;

  let mut info = Vec::new();
  write_i32(&mut info, 1); // Количество аккаунтов
  write_i32(&mut info, 0); // Индекс аккаунта
  write_i32(&mut info, 0); // Активный аккаунт
  let info_encrypted = prepare_encrypted(&info, &local_key)?;

  let mut key_file = Vec::new();
  write_bytes(&mut key_file, Some(&salt));
  write_bytes(&mut key_file, Some(&key_encrypted));
  write_bytes(&mut key_file, Some(&info_encrypted));

  // MTP-авторизация
  let mut auth = Vec::new();
  write_u64(&mut auth, WIDE_IDS_TAG);
  write_u64(&mut auth, session.user_id);
  write_i32(&mut auth, session.main_dc_id);
  write_i32(&mut auth, session.keys.len() as i32);
  for (dc_id, key) in &session.keys {
    write_i32(&mut auth, *dc_id);
    auth.extend_from_slice(key);
  }
  write_i32(&mut auth, 0); // Ключи на удаление

  let mut mtp = Vec::new();
  write_u32(&mut mtp, DBI_MTP_AUTHORIZATION);
  write_bytes(&mut mtp, Some(&auth));
  let mut mtp_file = Vec::new();
  write_bytes(&mut mtp_file, Some(&prepare_encrypted(&mtp, &local_key)?));

  // Карта файлов аккаунта: две пустые legacy-записи и зашифрованная пустая карта
  let mut map_file = Vec::new();
  write_bytes(&mut map_file, None);
  write_bytes(&mut map_file, None);
  write_bytes(&mut map_file, Some(&prepare_encrypted(&[], &local_key)?));

  let account_part = to_file_part(ACCOUNT_DATA_NAME);
  let account_dir = dir.join(&account_part);
  fs::create_dir_all(&account_dir).map_err(|e| e.to_string())?;
  write_tdf(&dir.join(format!("key_{ACCOUNT_DATA_NAME}s")), &key_file)?;
  write_tdf(&dir.join(format!("{account_part}s")), &mtp_file)?;
  write_tdf(&account_dir.join("maps"), &map_file)?;
  Ok(())
}

pub fn parse_auth_key(hex: &str) -> Option<[u8; AUTH_KEY_SIZE]> {
  let hex = hex.trim();
  if hex.len() != AUTH_KEY_SIZE * 2 {
    return None;
  }
  let mut key = [0u8; AUTH_KEY_SIZE];
  for (i, byte) in key.iter_mut().enumerate() {
    *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
  }
  Some(key)
}

// --- Формат файлов ---

fn write_tdf(path: &Path, data: &[u8]) -> Result<(), String> {
  let mut md5 = Md5::new();
  md5.update(data);
  md5.update((data.len() as i32).to_le_bytes());
  md5.update(TDF_VERSION.to_le_bytes());
  md5.update(TDF_MAGIC);

  let mut out = Vec::with_capacity(data.len() + 24);
  out.extend_from_slice(TDF_MAGIC);
  out.extend_from_slice(&TDF_VERSION.to_le_bytes());
  out.extend_from_slice(data);
  out.extend_from_slice(&md5.finalize());
  fs::write(path, out).map_err(|e| e.to_string())
}

/// Имя файла tdesktop: первые 8 байт MD5 имени, по полубайтам начиная с младшего
fn to_file_part(name: &str) -> String {
  const HEX: &[u8; 16] = b"0123456789ABCDEF";
  let digest = Md5::digest(name.as_bytes());
  digest[..8]
    .iter()
    .flat_map(|b| [HEX[(b & 0x0F) as usize] as char, HEX[(b >> 4) as usize] as char])
    .collect()
}

// QDataStream: big-endian, QByteArray = u32 длина + байты (0xFFFFFFFF для null)
fn write_i32(buf: &mut Vec<u8>, v: i32) {
  buf.extend_from_slice(&v.to_be_bytes());
}

fn write_u32(buf: &mut Vec<u8>, v: u32) {
  buf.extend_from_slice(&v.to_be_bytes());
}

fn write_u64(buf: &mut Vec<u8>, v: u64) {
  buf.extend_from_slice(&v.to_be_bytes());
}

fn write_bytes(buf: &mut Vec<u8>, data: Option<&[u8]>) {
  match data {
    Some(data) => {
      write_u32(buf, data.len() as u32);
      buf.extend_from_slice(data);
    }
    None => write_u32(buf, u32::MAX),
  }
}

// --- Криптография tdesktop ---

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
  let mut buf = [0u8; N];
  getrandom::fill(&mut buf).map_err(|e| e.to_string())?;
  Ok(buf)
}

/// `CreateLocalKey`: PBKDF2-HMAC-SHA512 от SHA512(salt + passcode + salt)
fn create_local_key(passcode: &[u8], salt: &[u8]) -> [u8; AUTH_KEY_SIZE] {
  let mut hasher = Sha512::new();
  hasher.update(salt);
  hasher.update(passcode);
  hasher.update(salt);
  let hash = hasher.finalize();
  let mut key = [0u8; AUTH_KEY_SIZE];
  pbkdf2::pbkdf2_hmac::<Sha512>(&hash, salt, PASSCODE_ITERATIONS, &mut key);
  key
}

/// `PrepareEncrypted`: [u32 LE размер][данные][случайное выравнивание до 16] → msg_key(16) + AES-IGE
fn prepare_encrypted(payload: &[u8], key: &[u8; AUTH_KEY_SIZE]) -> Result<Vec<u8>, String> {
  let size = 4 + payload.len();
  let full_size = size.div_ceil(16) * 16;
  let mut plain = Vec::with_capacity(full_size);
  plain.extend_from_slice(&(size as u32).to_le_bytes());
  plain.extend_from_slice(payload);
  let mut padding = vec![0u8; full_size - size];
  getrandom::fill(&mut padding).map_err(|e| e.to_string())?;
  plain.extend_from_slice(&padding);

  let msg_key: [u8; 16] = Sha1::digest(&plain)[..16].try_into().unwrap();
  let (aes_key, aes_iv) = prepare_aes_oldmtp(key, &msg_key);
  let mut out = msg_key.to_vec();
  out.extend_from_slice(&aes_ige_encrypt(&plain, &aes_key, &aes_iv));
  Ok(out)
}

/// `AuthKey::prepareAES_oldmtp` с `send = false` (смещение 8)
fn prepare_aes_oldmtp(key: &[u8; AUTH_KEY_SIZE], msg_key: &[u8; 16]) -> ([u8; 32], [u8; 32]) {
  const X: usize = 8;
  let sha = |parts: &[&[u8]]| {
    let mut h = Sha1::new();
    for p in parts {
      h.update(p);
    }
    let out: [u8; 20] = h.finalize().into();
    out
  };
  let a = sha(&[msg_key, &key[X..X + 32]]);
  let b = sha(&[&key[32 + X..48 + X], msg_key, &key[48 + X..64 + X]]);
  let c = sha(&[&key[64 + X..96 + X], msg_key]);
  let d = sha(&[msg_key, &key[96 + X..128 + X]]);

  let mut aes_key = [0u8; 32];
  aes_key[..8].copy_from_slice(&a[..8]);
  aes_key[8..20].copy_from_slice(&b[8..20]);
  aes_key[20..32].copy_from_slice(&c[4..16]);

  let mut aes_iv = [0u8; 32];
  aes_iv[..12].copy_from_slice(&a[8..20]);
  aes_iv[12..20].copy_from_slice(&b[..8]);
  aes_iv[20..24].copy_from_slice(&c[16..20]);
  aes_iv[24..32].copy_from_slice(&d[..8]);
  (aes_key, aes_iv)
}

/// AES-256-IGE: iv[0..16] — предыдущий шифроблок, iv[16..32] — предыдущий открытый блок
fn aes_ige_encrypt(plain: &[u8], key: &[u8; 32], iv: &[u8; 32]) -> Vec<u8> {
  let cipher = Aes256::new(GenericArray::from_slice(key));
  let mut prev_cipher: [u8; 16] = iv[..16].try_into().unwrap();
  let mut prev_plain: [u8; 16] = iv[16..].try_into().unwrap();
  let mut out = Vec::with_capacity(plain.len());
  for chunk in plain.chunks(16) {
    let mut block = [0u8; 16];
    for i in 0..16 {
      block[i] = chunk[i] ^ prev_cipher[i];
    }
    let mut ga = GenericArray::from(block);
    cipher.encrypt_block(&mut ga);
    for i in 0..16 {
      ga[i] ^= prev_plain[i];
    }
    prev_plain.copy_from_slice(chunk);
    prev_cipher.copy_from_slice(&ga);
    out.extend_from_slice(&ga);
  }
  out
}

#[cfg_attr(not(test), allow(dead_code))]
fn aes_ige_decrypt(data: &[u8], key: &[u8; 32], iv: &[u8; 32]) -> Vec<u8> {
  let cipher = Aes256::new(GenericArray::from_slice(key));
  let mut prev_cipher: [u8; 16] = iv[..16].try_into().unwrap();
  let mut prev_plain: [u8; 16] = iv[16..].try_into().unwrap();
  let mut out = Vec::with_capacity(data.len());
  for chunk in data.chunks(16) {
    let mut block = [0u8; 16];
    for i in 0..16 {
      block[i] = chunk[i] ^ prev_plain[i];
    }
    let mut ga = GenericArray::from(block);
    cipher.decrypt_block(&mut ga);
    for i in 0..16 {
      ga[i] ^= prev_cipher[i];
    }
    prev_cipher.copy_from_slice(chunk);
    prev_plain.copy_from_slice(&ga);
    out.extend_from_slice(&ga);
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  // Повторяет `DecryptLocal` из tdesktop
  fn decrypt_local(encrypted: &[u8], key: &[u8; AUTH_KEY_SIZE]) -> Vec<u8> {
    let msg_key: [u8; 16] = encrypted[..16].try_into().unwrap();
    let (aes_key, aes_iv) = prepare_aes_oldmtp(key, &msg_key);
    let plain = aes_ige_decrypt(&encrypted[16..], &aes_key, &aes_iv);
    assert_eq!(&Sha1::digest(&plain)[..16], &msg_key, "msg_key mismatch");
    let size = u32::from_le_bytes(plain[..4].try_into().unwrap()) as usize;
    assert!(size >= 4 && size <= plain.len() && size > plain.len() - 16);
    plain[4..size].to_vec()
  }

  fn read_tdf(path: &Path) -> Vec<u8> {
    let raw = fs::read(path).unwrap();
    assert_eq!(&raw[..4], TDF_MAGIC);
    let data = &raw[8..raw.len() - 16];
    let mut md5 = Md5::new();
    md5.update(data);
    md5.update((data.len() as i32).to_le_bytes());
    md5.update(&raw[4..8]);
    md5.update(TDF_MAGIC);
    assert_eq!(&md5.finalize()[..], &raw[raw.len() - 16..], "md5 mismatch");
    data.to_vec()
  }

  fn read_bytes(buf: &[u8], pos: &mut usize) -> Vec<u8> {
    let len = u32::from_be_bytes(buf[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    if len == u32::MAX {
      return Vec::new();
    }
    let out = buf[*pos..*pos + len as usize].to_vec();
    *pos += len as usize;
    out
  }

  #[test]
  fn file_part_matches_tdesktop() {
    assert_eq!(to_file_part("data"), "D877F783D5D3EF8C");
  }

  #[test]
  fn ige_roundtrip() {
    let key = [7u8; 32];
    let iv = [9u8; 32];
    let plain: Vec<u8> = (0..64).collect();
    assert_eq!(aes_ige_decrypt(&aes_ige_encrypt(&plain, &key, &iv), &key, &iv), plain);
  }

  #[test]
  fn writes_readable_tdata() {
    let dir = std::env::temp_dir().join(format!("taurigram-tdata-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let session = Session { user_id: 123_456_789, main_dc_id: 2, keys: vec![(2, [0xAB; AUTH_KEY_SIZE]), (4, [0xCD; AUTH_KEY_SIZE])] };
    write(&dir, &session).unwrap();

    // key_datas: соль, локальный ключ, список аккаунтов
    let key_file = read_tdf(&dir.join("key_datas"));
    let mut pos = 0;
    let salt = read_bytes(&key_file, &mut pos);
    let key_encrypted = read_bytes(&key_file, &mut pos);
    let info_encrypted = read_bytes(&key_file, &mut pos);
    let passcode_key = create_local_key(&[], &salt);
    let local_key: [u8; AUTH_KEY_SIZE] = decrypt_local(&key_encrypted, &passcode_key).try_into().unwrap();
    let info = decrypt_local(&info_encrypted, &local_key);
    assert_eq!(info, [0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);

    // MTP-авторизация
    let mtp_file = read_tdf(&dir.join("D877F783D5D3EF8Cs"));
    let mut pos = 0;
    let mtp = decrypt_local(&read_bytes(&mtp_file, &mut pos), &local_key);
    assert_eq!(u32::from_be_bytes(mtp[..4].try_into().unwrap()), DBI_MTP_AUTHORIZATION);
    let mut pos = 4;
    let auth = read_bytes(&mtp, &mut pos);
    assert_eq!(&auth[..8], &[0xFF; 8]);
    assert_eq!(u64::from_be_bytes(auth[8..16].try_into().unwrap()), 123_456_789);
    assert_eq!(i32::from_be_bytes(auth[16..20].try_into().unwrap()), 2);
    assert_eq!(i32::from_be_bytes(auth[20..24].try_into().unwrap()), 2);
    assert_eq!(i32::from_be_bytes(auth[24..28].try_into().unwrap()), 2);
    assert_eq!(auth[28..28 + AUTH_KEY_SIZE], [0xAB; AUTH_KEY_SIZE]);
    assert_eq!(auth.len(), 8 + 8 + 4 + 4 + 2 * (4 + AUTH_KEY_SIZE) + 4);

    // Карта аккаунта
    let map = read_tdf(&dir.join("D877F783D5D3EF8C").join("maps"));
    let mut pos = 0;
    assert!(read_bytes(&map, &mut pos).is_empty());
    assert!(read_bytes(&map, &mut pos).is_empty());
    assert!(decrypt_local(&read_bytes(&map, &mut pos), &local_key).is_empty());

    let _ = fs::remove_dir_all(&dir);
  }
}
