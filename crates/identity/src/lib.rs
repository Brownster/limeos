use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::SaltString,
};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub fn opaque() -> std::io::Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| std::io::Error::other("secure randomness unavailable"))?;
    Ok(hex::encode(bytes))
}
pub fn digest(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
pub fn constant_eq(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

fn argon() -> Argon2<'static> {
    // 19 MiB, two iterations, one lane. At most two concurrent password workers.
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(19 * 1024, 2, 1, Some(32)).expect("fixed valid parameters"),
    )
}
pub fn hash(password: &str) -> Option<String> {
    if password.is_empty() || password.len() > 1024 {
        return None;
    }
    let mut salt = [0_u8; 16];
    getrandom::getrandom(&mut salt).ok()?;
    argon()
        .hash_password(password.as_bytes(), &SaltString::encode_b64(&salt).ok()?)
        .ok()
        .map(|h| h.to_string())
}

/// Verification is bounded even when the imported hash is hostile. Legacy salt is
/// raw UTF-8 (Werkzeug), whereas PHC salts are base64. Never log either input.
pub fn verify(password: &str, encoded: &str) -> bool {
    if password.len() > 1024 || encoded.len() > 512 {
        return false;
    }
    if encoded.starts_with("$argon2id$") {
        let Ok(parsed) = PasswordHash::new(encoded) else {
            return false;
        };
        let Ok(params) = Params::try_from(&parsed) else {
            return false;
        };
        if params.m_cost() > 64 * 1024 || params.t_cost() > 6 || params.p_cost() > 4 {
            return false;
        }
        return argon()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok();
    }
    let Some((method, salt, expected)) = legacy(encoded) else {
        return false;
    };
    match method {
        Legacy::Pbkdf2(count) => std::num::NonZeroU32::new(count).is_some_and(|iterations| {
            ring::pbkdf2::verify(
                ring::pbkdf2::PBKDF2_HMAC_SHA256,
                iterations,
                salt.as_bytes(),
                password.as_bytes(),
                &expected,
            )
            .is_ok()
        }),
        Legacy::Scrypt(params) => {
            let mut actual = vec![0_u8; expected.len()];
            if scrypt::scrypt(password.as_bytes(), salt.as_bytes(), &params, &mut actual).is_err() {
                return false;
            }
            actual.ct_eq(&expected).into()
        }
    }
}
enum Legacy {
    Pbkdf2(u32),
    Scrypt(scrypt::Params),
}
fn legacy(encoded: &str) -> Option<(Legacy, &str, Vec<u8>)> {
    if encoded.len() > 512 {
        return None;
    }
    let fields: Vec<_> = encoded.split('$').collect();
    if fields.len() != 3 || fields[1].is_empty() || fields[1].len() > 128 {
        return None;
    }
    let expected = hex::decode(fields[2]).ok()?;
    let method: Vec<_> = fields[0].split(':').collect();
    let parsed = match method.as_slice() {
        ["pbkdf2", "sha256", iterations] if expected.len() == 32 => {
            let count = iterations.parse::<u32>().ok()?;
            if !(1..=2_000_000).contains(&count) {
                return None;
            }
            Legacy::Pbkdf2(count)
        }
        ["scrypt", n, r, p] if expected.len() == 64 => {
            let (n, r, p) = (
                n.parse::<u32>().ok()?,
                r.parse::<u32>().ok()?,
                p.parse::<u32>().ok()?,
            );
            if !n.is_power_of_two()
                || !(2..=32768).contains(&n)
                || !(1..=8).contains(&r)
                || !(1..=2).contains(&p)
            {
                return None;
            }
            Legacy::Scrypt(scrypt::Params::new(n.ilog2() as u8, r, p, 64).ok()?)
        }
        _ => return None,
    };
    Some((parsed, fields[1], expected))
}
pub fn supported_legacy(encoded: &str) -> bool {
    legacy(encoded).is_some()
}
pub fn needs_upgrade(encoded: &str) -> bool {
    !encoded.starts_with("$argon2id$")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verifies_hashes_generated_by_werkzeug() {
        let fixtures: serde_json::Value =
            serde_json::from_str(include_str!("../../../tests/fixtures/werkzeug-hashes.json"))
                .unwrap();
        for row in fixtures.as_array().unwrap() {
            let password = row["password"].as_str().unwrap();
            let encoded = row["hash"].as_str().unwrap();
            assert!(verify(password, encoded));
            assert!(!verify("wrong-password", encoded));
            let upgraded = hash(password).unwrap();
            assert!(!needs_upgrade(&upgraded));
            assert!(verify(password, &upgraded));
        }
    }
    #[test]
    fn argon_round_trip_wrong_password_and_bounded_hostile_parameters() {
        let encoded = hash("correct horse battery staple").unwrap();
        assert!(verify("correct horse battery staple", &encoded));
        assert!(!verify("wrong", &encoded));
        assert!(!verify(
            "wrong",
            "$argon2id$v=19$m=999999999,t=2,p=1$YWJjZGVmZ2g$YWJjZGVmZ2g"
        ));
        assert!(!verify("wrong", "scrypt:4294967295:8:1$salt$00"));
        assert!(hash("").is_none());
    }
}
