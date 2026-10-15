// Single-user authentication: an Argon2id password plus optional passkeys (WebAuthn).
// The account lives in data/auth.json. On first start no account exists: a one-time
// setup code is printed in the logs and must be entered to create it, so nobody
// else reaching a fresh instance can claim it first.
// Delete data/auth.json to reset the account.

use std::collections::HashMap;
use std::fs;
use std::sync::{Mutex, RwLock};

use anyhow::{Context, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use serde::{Deserialize, Serialize};
use webauthn_rs::prelude::*;

use crate::state::now;

const AUTH_FILE: &str = "data/auth.json";
pub const SESSION_TTL: u64 = 14 * 24 * 3600;
const CEREMONY_TTL: u64 = 300;

#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    pub username: String,
    password_hash: String,
    user_id: Uuid,
    #[serde(default)]
    pub passkeys: Vec<NamedPasskey>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NamedPasskey {
    pub id: String,
    pub name: String,
    pub created: u64,
    passkey: Passkey,
}

pub struct Auth {
    account: Mutex<Option<Account>>,
    pub setup_code: String,
    /// token -> expiry
    sessions: Mutex<HashMap<String, u64>>,
    /// Rebuilt when `public_url` changes from the web interface.
    webauthn: RwLock<Option<Webauthn>>,
    /// session token -> pending registration
    registrations: Mutex<HashMap<String, (PasskeyRegistration, u64)>>,
    /// ceremony id -> pending login
    logins: Mutex<HashMap<String, (PasskeyAuthentication, u64)>>,
}

pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("OS random generator unavailable");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Constant-time string comparison for secrets.
pub fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn hash_password(password: &str) -> Result<String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).expect("OS random generator unavailable");
    let salt = SaltString::encode_b64(&salt).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("{e}"))?
        .to_string())
}

fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash)
        .map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
        .unwrap_or(false)
}

fn build_webauthn(public_url: &str) -> Option<Webauthn> {
    let origin = Url::parse(public_url).ok()?;
    let rp_id = origin.domain()?.to_string();
    match WebauthnBuilder::new(&rp_id, &origin).and_then(|b| b.rp_name("RustFeed").build()) {
        Ok(w) => Some(w),
        Err(err) => {
            eprintln!("Passkeys disabled: `public_url` {public_url} is not usable for WebAuthn ({err}).");
            None
        }
    }
}

impl Auth {
    pub fn new(public_url: &str) -> Self {
        let account: Option<Account> = fs::read_to_string(AUTH_FILE)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok());
        let setup_code = random_token()[..12].to_string();
        if account.is_none() {
            println!("No account yet. Open the web interface and use this setup code: {setup_code}");
        }
        Self {
            account: Mutex::new(account),
            setup_code,
            sessions: Mutex::new(HashMap::new()),
            webauthn: RwLock::new(build_webauthn(public_url)),
            registrations: Mutex::new(HashMap::new()),
            logins: Mutex::new(HashMap::new()),
        }
    }

    pub fn webauthn(&self) -> Option<Webauthn> {
        self.webauthn.read().unwrap().clone()
    }

    /// Returns false when the address can't be used for passkeys (IP, invalid URL...).
    pub fn set_public_url(&self, public_url: &str) -> bool {
        let built = build_webauthn(public_url);
        let ok = built.is_some();
        *self.webauthn.write().unwrap() = built;
        ok
    }

    pub fn account(&self) -> Option<Account> {
        self.account.lock().unwrap().clone()
    }

    fn store(&self, account: Account) -> Result<()> {
        fs::create_dir_all("data")?;
        fs::write(AUTH_FILE, serde_json::to_string_pretty(&account)?).context("Failed to write data/auth.json")?;
        *self.account.lock().unwrap() = Some(account);
        Ok(())
    }

    // ----- account -----

    pub fn setup(&self, code: &str, username: &str, password: &str) -> Result<bool> {
        if self.account().is_some() || !same(code.trim(), &self.setup_code) {
            return Ok(false);
        }
        let mut id = [0u8; 16];
        getrandom::fill(&mut id).expect("OS random generator unavailable");
        self.store(Account {
            username: username.to_string(),
            password_hash: hash_password(password)?,
            user_id: Uuid::from_bytes(id),
            passkeys: Vec::new(),
        })?;
        Ok(true)
    }

    /// Checks credentials. Runs the hash even for an unknown user so timing
    /// doesn't reveal whether the username exists.
    pub fn check_password(&self, username: &str, password: &str) -> bool {
        static DUMMY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        match self.account() {
            Some(acc) => {
                let ok = verify_password(&acc.password_hash, password);
                ok && same(&acc.username, username)
            }
            None => {
                verify_password(DUMMY.get_or_init(|| hash_password(&random_token()).unwrap_or_default()), password);
                false
            }
        }
    }

    pub fn change_password(&self, current: &str, new: &str) -> Result<bool> {
        let Some(mut acc) = self.account() else { return Ok(false) };
        if !verify_password(&acc.password_hash, current) {
            return Ok(false);
        }
        acc.password_hash = hash_password(new)?;
        self.store(acc)?;
        // Everyone else gets logged out.
        self.sessions.lock().unwrap().clear();
        Ok(true)
    }

    // ----- sessions -----

    pub fn create_session(&self) -> String {
        let token = random_token();
        let mut sessions = self.sessions.lock().unwrap();
        let t = now();
        sessions.retain(|_, exp| *exp > t);
        sessions.insert(token.clone(), t + SESSION_TTL);
        token
    }

    pub fn valid_session(&self, token: &str) -> bool {
        self.sessions.lock().unwrap().get(token).is_some_and(|exp| *exp > now())
    }

    pub fn end_session(&self, token: &str) {
        self.sessions.lock().unwrap().remove(token);
    }

    // ----- passkeys -----

    pub fn start_registration(&self, session: &str) -> Result<CreationChallengeResponse> {
        let webauthn = self.webauthn().context("Passkeys indisponibles (adresse publique)")?;
        let acc = self.account().context("Aucun compte")?;
        let exclude = acc.passkeys.iter().map(|p| p.passkey.cred_id().clone()).collect();
        let (ccr, state) = webauthn.start_passkey_registration(acc.user_id, &acc.username, &acc.username, Some(exclude))?;
        self.registrations.lock().unwrap().insert(session.to_string(), (state, now() + CEREMONY_TTL));
        Ok(ccr)
    }

    pub fn finish_registration(&self, session: &str, name: &str, cred: &RegisterPublicKeyCredential) -> Result<()> {
        let webauthn = self.webauthn().context("Passkeys indisponibles")?;
        let (state, exp) = self.registrations.lock().unwrap().remove(session).context("Aucun enregistrement en cours")?;
        anyhow::ensure!(exp > now(), "Délai dépassé, recommencez.");
        let passkey = webauthn.finish_passkey_registration(cred, &state)?;
        let mut acc = self.account().context("Aucun compte")?;
        acc.passkeys.push(NamedPasskey {
            id: random_token()[..16].to_string(),
            name: if name.trim().is_empty() { "Passkey".into() } else { name.trim().chars().take(60).collect() },
            created: now(),
            passkey,
        });
        self.store(acc)
    }

    pub fn start_login(&self) -> Result<(String, RequestChallengeResponse)> {
        let webauthn = self.webauthn().context("Passkeys indisponibles")?;
        let acc = self.account().context("Aucun compte")?;
        anyhow::ensure!(!acc.passkeys.is_empty(), "Aucune passkey enregistrée.");
        let keys: Vec<Passkey> = acc.passkeys.iter().map(|p| p.passkey.clone()).collect();
        let (rcr, state) = webauthn.start_passkey_authentication(&keys)?;
        let id = random_token();
        let mut logins = self.logins.lock().unwrap();
        let t = now();
        logins.retain(|_, (_, exp)| *exp > t);
        logins.insert(id.clone(), (state, t + CEREMONY_TTL));
        Ok((id, rcr))
    }

    pub fn finish_login(&self, id: &str, cred: &PublicKeyCredential) -> Result<()> {
        let webauthn = self.webauthn().context("Passkeys indisponibles")?;
        let (state, exp) = self.logins.lock().unwrap().remove(id).context("Aucune connexion en cours")?;
        anyhow::ensure!(exp > now(), "Délai dépassé, recommencez.");
        let result = webauthn.finish_passkey_authentication(cred, &state)?;
        // Persist the updated signature counter (clone detection).
        let mut acc = self.account().context("Aucun compte")?;
        let mut changed = false;
        for p in &mut acc.passkeys {
            changed |= p.passkey.update_credential(&result).unwrap_or(false);
        }
        if changed {
            self.store(acc)?;
        }
        Ok(())
    }

    pub fn delete_passkey(&self, id: &str) -> Result<bool> {
        let Some(mut acc) = self.account() else { return Ok(false) };
        let before = acc.passkeys.len();
        acc.passkeys.retain(|p| p.id != id);
        if acc.passkeys.len() == before {
            return Ok(false);
        }
        self.store(acc)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_roundtrip() {
        let h = hash_password("correct horse").unwrap();
        assert!(verify_password(&h, "correct horse"));
        assert!(!verify_password(&h, "wrong"));
        assert!(same("abc", "abc") && !same("abc", "abd") && !same("abc", "ab"));
    }
}
