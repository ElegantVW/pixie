use crate::paths;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const CONTACT_MEMORY_SEED: &str = "# WhatsApp contact memory\n\nPrivate notes for this contact only. Store useful preferences and context, not secrets or assumptions. Keep messages and sensitive personal details out unless specifically asked.\n";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactProfile {
    pub jid: String,
    pub name: String,
}

impl ContactProfile {
    pub fn session_id(&self) -> String {
        format!("wa-contact-{:016x}", stable_hash(&self.jid))
    }

    pub fn memory_path(&self) -> PathBuf {
        contacts_dir().join(format!("{:016x}.md", stable_hash(&self.jid)))
    }

    pub fn save(&self) -> Result<()> {
        if !valid_direct_jid(&self.jid) {
            bail!("contact is not a one-to-one WhatsApp chat");
        }
        ensure_private_dir(&contacts_dir())?;
        let path = profile_path(&self.session_id());
        let contents = serde_json::to_vec_pretty(self)?;
        paths::write_atomic(&path, &contents)
            .with_context(|| format!("saving contact profile {}", path.display()))?;
        protect_private_file(&path)?;

        let memory = self.memory_path();
        if !memory.exists() {
            paths::write_atomic(&memory, CONTACT_MEMORY_SEED.as_bytes())
                .with_context(|| format!("creating contact memory {}", memory.display()))?;
        }
        protect_private_file(&memory)?;
        Ok(())
    }
}

pub fn resolve(query: &str) -> Result<ContactProfile> {
    let query = query.trim();
    if query.is_empty() {
        bail!("contact name is empty");
    }
    let output = Command::new("wacli")
        .args([
            "--json",
            "--read-only",
            "contacts",
            "search",
            query,
            "--limit",
            "50",
        ])
        .stdin(Stdio::null())
        .output()
        .context("could not run wacli contact lookup")?;
    if !output.status.success() {
        bail!("local WhatsApp contact lookup failed");
    }
    let result: Value = serde_json::from_slice(&output.stdout)
        .context("wacli contact lookup returned invalid JSON")?;
    if result.get("error").is_some_and(|error| !error.is_null()) {
        bail!("local WhatsApp contact lookup failed");
    }
    profile_from_response(&result, query)
}

fn contact_rows(response: &Value) -> Result<&[Value]> {
    response
        .pointer("/data")
        .and_then(Value::as_array)
        .or_else(|| response.pointer("/data/contacts").and_then(Value::as_array))
        .or_else(|| response.get("contacts").and_then(Value::as_array))
        .map(Vec::as_slice)
        .ok_or_else(|| anyhow::anyhow!("wacli returned no contact list"))
}

fn profile_from_response(response: &Value, query: &str) -> Result<ContactProfile> {
    let query_lower = query.trim().to_lowercase();
    let matches: Vec<_> = contact_rows(response)?
        .iter()
        .filter(|row| {
            ["name", "alias", "phone", "jid"]
                .iter()
                .filter_map(|field| row.get(*field).and_then(Value::as_str))
                .any(|value| value.trim().to_lowercase() == query_lower)
        })
        .collect();
    if matches.is_empty() {
        bail!("no exact local WhatsApp contact matches '{query}'");
    }
    if matches.len() != 1 {
        bail!("'{query}' matches multiple WhatsApp contacts; use an exact phone number");
    }
    let row = matches[0];
    let jid = row
        .get("jid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if !valid_direct_jid(&jid) {
        bail!("contact is not a one-to-one WhatsApp chat");
    }
    let name = row
        .get("name")
        .or_else(|| row.get("alias"))
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(query);
    let name = clean_display_name(name);
    if name.is_empty() {
        bail!("contact has no usable display name");
    }
    Ok(ContactProfile { jid, name })
}

pub fn for_session(session_id: &str) -> Option<ContactProfile> {
    if !is_contact_session(session_id) {
        return None;
    }
    let path = profile_path(session_id);
    let contents = fs::read(&path).ok()?;
    let profile: ContactProfile = serde_json::from_slice(&contents).ok()?;
    if profile.session_id() != session_id || !valid_direct_jid(&profile.jid) {
        return None;
    }
    let name = clean_display_name(&profile.name);
    if name.is_empty() {
        return None;
    }
    Some(ContactProfile { name, ..profile })
}

pub fn is_contact_session(session_id: &str) -> bool {
    let Some(hash) = session_id.strip_prefix("wa-contact-") else {
        return false;
    };
    hash.len() == 16 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn has_contact_session_prefix(session_id: &str) -> bool {
    session_id.starts_with("wa-contact-")
}

pub fn session_path_for_id(session_id: &str) -> Option<PathBuf> {
    is_contact_session(session_id).then(|| contacts_dir().join(format!("{session_id}.jsonl")))
}

pub fn constrain_tool(name: &str, args: &Value, profile: &ContactProfile) -> Result<Value, String> {
    if name == "remember" {
        return Ok(args.clone());
    }
    if name != "check_whatsapp" {
        return Err(format!("'{name}' is not available in a contact session"));
    }

    let mut scoped = args.clone();
    let object = scoped
        .as_object_mut()
        .ok_or_else(|| "WhatsApp arguments must be an object".to_string())?;
    let action = object
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("list")
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        action.as_str(),
        "list" | "search" | "show" | "send" | "send_file" | "send_voice"
    ) {
        return Err(format!(
            "WhatsApp action '{action}' is not available in a contact session"
        ));
    }
    object.insert("action".into(), Value::String(action.clone()));
    object.insert(
        "contact_name".into(),
        Value::String(clean_display_name(&profile.name)),
    );
    if matches!(action.as_str(), "list" | "search" | "show") {
        object.insert("chat".into(), Value::String(profile.jid.clone()));
    } else {
        let to = object
            .get("to")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let phone = profile
            .jid
            .strip_suffix("@s.whatsapp.net")
            .unwrap_or(&profile.jid);
        if !to.is_empty() && to != profile.jid && to != profile.name && to != phone {
            return Err("contact sessions can only send to the selected contact".into());
        }
        object.insert("to".into(), Value::String(profile.jid.clone()));
    }
    Ok(scoped)
}

fn contacts_dir() -> PathBuf {
    paths::data_dir().join("contacts")
}

fn valid_direct_jid(jid: &str) -> bool {
    jid.strip_suffix("@s.whatsapp.net").is_some_and(|number| {
        !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn clean_display_name(name: &str) -> String {
    name.chars()
        .filter(|character| !character.is_control())
        .take(100)
        .collect::<String>()
        .trim()
        .to_string()
}

fn profile_path(session_id: &str) -> PathBuf {
    contacts_dir().join(format!("{session_id}.json"))
}

pub fn contact_storage_dir() -> PathBuf {
    contacts_dir()
}

pub fn ensure_private_dir(path: &std::path::Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("creating private contact directory {}", path.display()))?;
    set_private_dir(path)
}

pub fn protect_private_file(path: &std::path::Path) -> Result<()> {
    set_private_file(path)
}

fn stable_hash(value: &str) -> u64 {
    value.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(unix)]
fn set_private_dir(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .with_context(|| format!("protecting contact context directory {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_dir(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .with_context(|| format!("protecting contact context file {}", path.display()))
}

#[cfg(not(unix))]
fn set_private_file(_path: &std::path::Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn resolves_only_exact_direct_contacts() {
        let response = json!({"data": [
            {"jid": "351@s.whatsapp.net", "name": "Ada"},
            {"jid": "123@g.us", "name": "Ada group"}
        ]});
        let profile = profile_from_response(&response, "ada").unwrap();
        assert_eq!(profile.name, "Ada");
        assert_eq!(profile.jid, "351@s.whatsapp.net");
        assert!(profile_from_response(&response, "Car").is_err());
    }

    #[test]
    fn rejects_ambiguous_names_and_group_chats() {
        let ambiguous = json!({"data": [
            {"jid": "351@s.whatsapp.net", "name": "Ada"},
            {"jid": "352@s.whatsapp.net", "name": "Ada", "alias": "C"}
        ]});
        assert!(profile_from_response(&ambiguous, "Ada").is_err());
        let group = json!({"data": [
            {"jid": "123@g.us", "name": "Ada"}
        ]});
        assert!(profile_from_response(&group, "Ada").is_err());
    }

    #[test]
    fn session_and_memory_are_contact_scoped() {
        let a = ContactProfile {
            jid: "351@s.whatsapp.net".into(),
            name: "Ada".into(),
        };
        let b = ContactProfile {
            jid: "352@s.whatsapp.net".into(),
            name: "Other".into(),
        };
        assert_ne!(a.session_id(), b.session_id());
        assert_ne!(a.memory_path(), b.memory_path());
        assert!(a.session_id().starts_with("wa-contact-"));
        assert!(is_contact_session(&a.session_id()));
        assert!(!is_contact_session("wa-contact-../../etc/passwd"));
        assert!(has_contact_session_prefix("wa-contact-invalid"));
    }

    #[test]
    fn scopes_reads_sends_and_denies_other_actions() {
        let profile = ContactProfile {
            jid: "351@s.whatsapp.net".into(),
            name: "Ada".into(),
        };
        let read =
            constrain_tool("check_whatsapp", &json!({"action": "search"}), &profile).unwrap();
        assert_eq!(read["chat"], profile.jid);

        let send = constrain_tool(
            "check_whatsapp",
            &json!({"action": "send", "to": "Ada", "text": "hello"}),
            &profile,
        )
        .unwrap();
        assert_eq!(send["to"], profile.jid);
        assert_eq!(send["contact_name"], profile.name);
        assert!(constrain_tool(
            "check_whatsapp",
            &json!({"action": "send", "to": "other@s.whatsapp.net"}),
            &profile,
        )
        .is_err());
        assert!(constrain_tool("check_whatsapp", &json!({"action": "sync"}), &profile).is_err());
        assert!(constrain_tool("execute_bash", &json!({}), &profile).is_err());
    }
}
