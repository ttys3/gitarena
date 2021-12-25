use crate::sso::sso_provider_type::SSOProviderType;

use sqlx::FromRow;

pub(crate) mod github_sso;
pub(crate) mod sso_provider;
pub(crate) mod sso_provider_type;

#[derive(FromRow)]
pub(crate) struct SSO {
    pub(crate) user_id: i32, // User id on our end
    pub(crate) provider: SSOProviderType,
    pub(crate) provider_id: i32 // User id on the provider end
}
