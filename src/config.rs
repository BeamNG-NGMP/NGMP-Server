use serde::{Serialize, Deserialize};

pub fn load_config() -> Config {
    match std::fs::read_to_string("server_config.toml") {
        Ok(content) => {
            match toml::from_str::<Config>(&content) {
                Ok(config) => return config,
                Err(e) => panic!("failed to parse server_config.toml: {}", e),
            }
        },
        Err(e) => panic!("failed to read server_config.toml: {}", e),
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Config {
    #[serde(rename = "General")]
    pub general: ConfigGeneral,
    #[serde(rename = "Networking")]
    pub networking: ConfigNetworking,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigGeneral {
    pub map: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ConfigNetworking {
    pub tcp_port: u16,
    pub udp_port: u16,
    pub http_port: u16,
}
