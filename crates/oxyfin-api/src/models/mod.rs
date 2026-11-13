//! Data transfer objects returned by the Jellyfin API.

mod system;
mod user;

pub use system::PublicSystemInfo;
pub(crate) use user::AuthenticationResult;
pub use user::UserDto;
