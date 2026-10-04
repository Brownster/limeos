use limeos_contracts::{PasswordRequest, PasswordResponse};
use std::io::{Read, Write};
fn run() -> Option<()> {
    let mut input = String::new();
    std::io::stdin()
        .take(16385)
        .read_to_string(&mut input)
        .ok()?;
    if input.len() > 16384 {
        return None;
    }
    let request: PasswordRequest = serde_json::from_str(&input).ok()?;
    let output = match request {
        PasswordRequest::Hash { password } => PasswordResponse {
            valid: true,
            upgraded: Some(limeos_identity::hash(&password)?),
        },
        PasswordRequest::Verify { password, encoded } => {
            let valid = limeos_identity::verify(&password, &encoded);
            let upgraded = if valid && limeos_identity::needs_upgrade(&encoded) {
                Some(limeos_identity::hash(&password)?)
            } else {
                None
            };
            PasswordResponse { valid, upgraded }
        }
    };
    std::io::stdout()
        .write_all(&serde_json::to_vec(&output).ok()?)
        .ok()?;
    Some(())
}
fn main() {
    if run().is_none() {
        std::process::exit(1);
    }
}
