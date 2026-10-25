use crate::{
    credentials::CredentialState,
    ipc::{AppError, ErrorCode},
};
use serde::{Deserialize, Serialize};
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Selection {
    pub profile_id: String,
    pub expected_revision: i64,
    pub model_id: String,
    #[serde(deserialize_with = "required_option")]
    pub effort: Option<String>,
}
impl Selection {
    pub fn validate(&self) -> Result<(), ErrorCode> {
        if !valid_id(&self.profile_id)
            || !(1..=super::MAX_REVISION).contains(&self.expected_revision)
            || !valid_model(&self.model_id)
            || self.effort.as_ref().is_some_and(|v| !valid_model(v))
        {
            return Err(ErrorCode::ProviderSelectionInvalid);
        }
        Ok(())
    }
}
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}
pub fn valid_model(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub source_kind: String,
    pub adapter_kind: String,
    pub endpoint: String,
    pub wire_api: String,
    pub auth_kind: String,
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub enabled: bool,
    pub config_revision: i64,
    pub credential: CredentialState,
    pub available: bool,
    pub unavailable_code: Option<&'static str>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Save {
    #[serde(deserialize_with = "required_option")]
    pub profile_id: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub expected_revision: Option<i64>,
    pub name: String,
    pub endpoint: String,
    #[serde(deserialize_with = "required_option")]
    pub default_model: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub default_effort: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProfileRequest {
    pub profile_id: String,
    pub expected_revision: i64,
}
impl ProfileRequest {
    pub fn validate(&self) -> Result<(), ErrorCode> {
        if !valid_id(&self.profile_id)
            || !(1..=super::MAX_REVISION).contains(&self.expected_revision)
        {
            Err(ErrorCode::ProviderSelectionInvalid)
        } else {
            Ok(())
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Enabled {
    pub profile_id: String,
    pub expected_revision: i64,
    pub enabled: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelsRequest {
    pub profile_id: String,
    pub expected_revision: i64,
    pub refresh: bool,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub model_id: String,
    pub efforts: Vec<String>,
    pub supports_default_effort: bool,
    pub context_window: Option<i64>,
    pub source: String,
    pub source_version: String,
    pub profile_revision: i64,
    pub observed_at: i64,
    pub expires_at: i64,
}
#[derive(Serialize)]
pub struct Models {
    pub capabilities: Vec<Capability>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderList {
    pub profiles: Vec<Profile>,
    pub active: Option<Selection>,
    pub config_state: String,
}
#[derive(Serialize)]
pub struct Removed {
    pub removed: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub operation_id: String,
    pub state: &'static str,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationRequest {
    pub operation_id: String,
}
#[derive(Serialize)]
pub struct Accepted {
    pub accepted: bool,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTest {
    pub tested_revision: i64,
    pub tested_at: i64,
    pub authenticated: bool,
    pub model_accepted: bool,
    pub effort_accepted: bool,
    pub evidence: &'static str,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SwitchResult {
    pub outcome: &'static str,
    pub active: Option<Selection>,
    pub config_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<AppError>,
}
#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum OperationResult {
    Test(ConnectionTest),
    Switch(SwitchResult),
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationView {
    pub operation_id: String,
    pub phase: String,
    pub result: Option<OperationResult>,
    pub error: Option<AppError>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublicSnapshot {
    pub id: String,
    pub profile_id: String,
    pub profile_name: String,
    pub source_kind: String,
    pub endpoint: String,
    pub model_id: String,
    pub effort: Option<String>,
    pub config_revision: i64,
    pub credential_revision: i64,
    pub effective_at: i64,
    pub routing_scope: &'static str,
}
