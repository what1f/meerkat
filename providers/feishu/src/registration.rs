//! Feishu Personal Agent application registration device flow.

use crate::http::{ClientError, Method, Request, Transport, encode_query_component};
use serde::Deserialize;
use std::time::Duration;

const REGISTRATION_PATH: &str = "/oauth/v1/app/registration";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Registration {
    pub device_code: String,
    pub verification_uri: String,
    pub user_code: String,
    pub interval: Duration,
    pub expires_in: Duration,
}

pub struct RegistrationCredential {
    pub app_id: String,
    pub creator_open_id: String,
    app_secret: String,
}

impl std::fmt::Debug for RegistrationCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RegistrationCredential")
            .field("app_id", &self.app_id)
            .field("creator_open_id", &self.creator_open_id)
            .field("app_secret", &"<redacted>")
            .finish()
    }
}

impl RegistrationCredential {
    pub fn app_secret(&self) -> &str {
        &self.app_secret
    }
}

#[derive(Debug)]
pub enum RegistrationPoll {
    Pending { slow_down: bool },
    Complete(RegistrationCredential),
    Denied,
    Expired,
}

#[derive(Clone)]
pub struct Registrar<T> {
    transport: T,
    accounts_base: String,
    verification_base: String,
}

impl<T: Transport> Registrar<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            accounts_base: "https://accounts.feishu.cn".into(),
            verification_base: "https://open.feishu.cn".into(),
        }
    }

    pub fn start(&self) -> Result<Registration, ClientError> {
        #[derive(Deserialize)]
        struct Response {
            #[serde(default)]
            device_code: String,
            #[serde(default)]
            verification_uri: String,
            #[serde(default)]
            verification_uri_complete: String,
            #[serde(default)]
            user_code: String,
            #[serde(default)]
            interval: u64,
            #[serde(default, rename = "expire_in")]
            expires_in: u64,
        }
        let (status, response): (u16, Response) = self.post(&[
            ("action", "begin"),
            ("archetype", "PersonalAgent"),
            ("auth_method", "client_secret"),
            ("request_user_info", "open_id tenant_brand"),
        ])?;
        if !(200..300).contains(&status) {
            return Err(ClientError::HttpStatus {
                status,
                operation: "Feishu app registration".into(),
            });
        }
        if response.device_code.is_empty() {
            return Err(ClientError::MissingField {
                operation: "Feishu app registration".into(),
                field: "device_code".into(),
            });
        }
        let verification_uri = if !response.verification_uri_complete.is_empty() {
            response.verification_uri_complete
        } else if !response.user_code.is_empty() {
            format!(
                "{}/page/cli?user_code={}",
                self.verification_base,
                encode_query_component(&response.user_code)
            )
        } else {
            response.verification_uri
        };
        if verification_uri.is_empty() {
            return Err(ClientError::MissingField {
                operation: "Feishu app registration".into(),
                field: "verification_uri".into(),
            });
        }
        Ok(Registration {
            device_code: response.device_code,
            verification_uri,
            user_code: response.user_code,
            interval: Duration::from_secs(response.interval.max(1)),
            expires_in: Duration::from_secs(response.expires_in.max(60)),
        })
    }

    pub fn poll(&self, device_code: &str) -> Result<RegistrationPoll, ClientError> {
        #[derive(Default, Deserialize)]
        struct UserInfo {
            #[serde(default)]
            open_id: String,
        }
        #[derive(Default, Deserialize)]
        struct Response {
            #[serde(default)]
            client_id: String,
            #[serde(default)]
            client_secret: String,
            #[serde(default)]
            user_info: UserInfo,
            #[serde(default)]
            error: String,
            #[serde(default)]
            error_description: String,
        }
        if device_code.trim().is_empty() {
            return Err(ClientError::InvalidInput("device code is required".into()));
        }
        let (status, response): (u16, Response) =
            self.post(&[("action", "poll"), ("device_code", device_code)])?;
        if !(200..300).contains(&status) && response.error.is_empty() {
            return Err(ClientError::HttpStatus {
                status,
                operation: "Feishu app registration".into(),
            });
        }
        match response.error.as_str() {
            "authorization_pending" => Ok(RegistrationPoll::Pending { slow_down: false }),
            "slow_down" => Ok(RegistrationPoll::Pending { slow_down: true }),
            "access_denied" => Ok(RegistrationPoll::Denied),
            "expired_token" => Ok(RegistrationPoll::Expired),
            "" if !response.client_id.is_empty() && !response.client_secret.is_empty() => {
                Ok(RegistrationPoll::Complete(RegistrationCredential {
                    app_id: response.client_id,
                    app_secret: response.client_secret,
                    creator_open_id: response.user_info.open_id,
                }))
            }
            "" => Ok(RegistrationPoll::Pending { slow_down: false }),
            _ => Err(ClientError::InvalidInput(
                if response.error_description.is_empty() {
                    "Feishu application registration failed".into()
                } else {
                    response.error_description
                },
            )),
        }
    }

    fn post<R: for<'de> Deserialize<'de>>(
        &self,
        fields: &[(&str, &str)],
    ) -> Result<(u16, R), ClientError> {
        let body = fields
            .iter()
            .map(|(key, value)| {
                format!(
                    "{}={}",
                    encode_query_component(key),
                    encode_query_component(value)
                )
            })
            .collect::<Vec<_>>()
            .join("&");
        let response = self.transport.execute(
            Request::new(
                Method::Post,
                format!("{}{}", self.accounts_base, REGISTRATION_PATH),
            )
            .body(body, "application/x-www-form-urlencoded"),
        )?;
        Ok((response.status, response.json()?))
    }
}
