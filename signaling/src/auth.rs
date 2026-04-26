use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorClaims {
    pub session_id: String,
    pub nickname: String,
    pub signaling_instance_id: String,
    pub intent: String,
    pub room_id: Option<String>,
    pub iat: u64,
    pub exp: u64,
}

#[derive(Debug, Error)]
pub enum TokenError {
    #[error("invalid connector token")]
    Invalid(#[from] jsonwebtoken::errors::Error),
    #[error("connector token was issued for another signaling instance")]
    WrongInstance,
}

pub fn validate_connector_token(
    token: &str,
    secret: &str,
    expected_instance_id: &str,
) -> Result<ConnectorClaims, TokenError> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;

    let data = decode::<ConnectorClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )?;
    if data.claims.signaling_instance_id != expected_instance_id {
        return Err(TokenError::WrongInstance);
    }

    Ok(data.claims)
}
