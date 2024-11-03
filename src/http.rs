use serde::{Serialize, Deserialize};

// const LOGIN_API: &'static str = "http://login.ngmp.net:11281";
const LOGIN_API: &'static str = "http://138.201.33.234:11281";

#[derive(Debug, Serialize, Deserialize)]
pub struct UserAuth {
    pub auth: String,
    pub steam_id: u64,
    pub user: User,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    pub name: String,
    pub avatar_hash: String,
}

pub async fn auth_token_get_steam_info(auth_token: &str) -> anyhow::Result<UserAuth> {
    let endpoint = format!("{LOGIN_API}/login_auth/{}", auth_token);

    let client = reqwest::Client::new();
    Ok(client.get(endpoint)
        .send()
        .await?
        .json()
        .await?)
}
