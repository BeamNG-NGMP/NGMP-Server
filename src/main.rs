#[macro_use]
extern crate log;

use tokio::sync::mpsc;

use ngmp_protocol_impl::server_launcher::Packet;
use ngmp_protocol_impl::{connection::*, server_launcher};

mod config;
mod data;
mod http;
mod logger;
mod plugin;
mod server;

use config::Config;
use server::*;

fn client_accept_thread(config: Config, tx: mpsc::Sender<Client>) {
    info!("Client accept thread launched!");

    let tx2 = tx.clone();
    let rt = tokio::runtime::Runtime::new().expect("Failed to spawn client accept runtime!");
    let handle = rt.spawn(async move {
        client_accept_async(config, tx).await;
    });

    loop {
        if tx2.is_closed() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    handle.abort();
    info!("Client accept thread killed.");
}

/// Handles the accepting of a client
async fn accept_client(
    mut tcp_conn: TcpConnection<Packet>,
    addr: std::net::SocketAddr,
    config: &Config,
) -> Option<Client> {
    // Handle client version
    let packet = tcp_conn
        .wait_for_packet()
        .await
        .map_err(|e| error!("Client error: {}", e))
        .ok()?;
    // TODO: Actually use version data to see if we are on the same protocol version
    let (version, confirm_id) = match packet {
        Packet::Version(p) => (p.client_version, p.confirm_id),
        _ => {
            error!("Incorrect packet sent: {:?}", packet);
            return None;
        }
    };
    debug!("Client version: {}", version);
    // Confirm client version
    tcp_conn
        .write_packet(&Packet::Confirmation(
            server_launcher::generic::ConfirmationPacket { confirm_id },
        ))
        .await
        .map_err(|e| error!("Client error: {}", e))
        .ok()?;

    // Authentication packet
    // TODO: Actually use auth code to verify client identity!
    let packet = tcp_conn
        .wait_for_packet()
        .await
        .map_err(|e| error!("Client error: {}", e))
        .ok()?;
    let auth_data = match packet {
        Packet::Authentication(p) => p,
        _ => {
            error!("Incorrect packet sent: {:?}", packet);
            return None;
        }
    };
    let user_info = match http::auth_token_get_steam_info(&auth_data.auth_code).await {
        Ok(user_info) => {
            // Confirm auth data
            tcp_conn
                .write_packet(&Packet::Confirmation(
                    server_launcher::generic::ConfirmationPacket {
                        confirm_id: auth_data.confirm_id,
                    },
                ))
                .await
                .map_err(|e| error!("Client error: {}", e))
                .ok()?;
            user_info
        }
        Err(e) => {
            error!("{}", e);
            // Kick client as we cannot authenticate them!
            tcp_conn
                .write_packet(&Packet::PlayerKick(
                    server_launcher::generic::PlayerKickPacket {
                        reason: String::from("Failed to authenticate!"),
                    },
                ))
                .await
                .map_err(|e| error!("Client error: {}", e))
                .ok()?;
            return None;
        }
    };

    // Send server info packet
    tcp_conn
        .write_packet(&Packet::ServerInfo(
            server_launcher::serverinfo::ServerInfoPacket {
                http_port: config.networking.http_port,
                udp_port: config.networking.udp_port,
            },
        ))
        .await
        .map_err(|e| error!("Client error: {}", e))
        .ok()?;

    // Determine UDP address
    let mut udp_addr = addr.clone();
    udp_addr.set_port(config.networking.udp_port + 1);
    debug!("UDP addr: {}", udp_addr);

    // LoadMap packet
    let confirm_id = 8; // TODO: Generate one randomly :);
    tcp_conn
        .write_packet(&Packet::LoadMap(
            server_launcher::serverinfo::LoadMapPacket {
                confirm_id,
                map_name: config.general.map.clone(),
            },
        ))
        .await
        .map_err(|e| error!("Client error: {}", e))
        .ok()?;

    // Now we must wait for the confirmation packet, confirming the launcher is done loading the map.
    let packet = tcp_conn
        .wait_for_packet()
        .await
        .map_err(|e| error!("Client error: {}", e))
        .ok()?;
    match packet {
        Packet::Confirmation(p) => {
            if p.confirm_id == confirm_id {
                Some(Client::new(
                    tcp_conn,
                    udp_addr,
                    user_info.steam_id,
                    user_info.user,
                ))
            } else {
                error!("Invalid confirmation ID");
                None
            }
        }
        _ => {
            error!("Incorrect packet sent: {:?}", packet);
            return None;
        }
    }
}

async fn client_accept_async(config: Config, tx: mpsc::Sender<Client>) {
    let tcp_addr = format!("0.0.0.0:{}", config.networking.tcp_port);
    let tcp_listener = tokio::net::TcpListener::bind(&tcp_addr)
        .await
        .expect("Failed to bind TCP socket!");

    loop {
        match tcp_listener.accept().await {
            Ok((socket, addr)) => {
                info!("New connection incoming from {}", addr);
                let tcp_conn = TcpConnection::<Packet>::from_stream(socket);
                if let Some(client) = accept_client(tcp_conn, addr, &config).await {
                    if let Err(e) = tx.send(client).await {
                        error!("Failed to send client over to server thread: {}", e);
                    }
                }
            }
            Err(e) => error!("Error accepting client: {}", e),
        }
    }
}

#[tokio::main]
async fn main() {
    logger::init(log::LevelFilter::max(), true).expect("Failed to initialize logger!");
    info!("Logger initialized!");

    let config = config::load_config();

    let udp_bind_addr = format!("0.0.0.0:{}", config.networking.udp_port);
    let udp_listener = UdpListener::<Packet>::bind(&udp_bind_addr)
        .await
        .map_err(|e| error!("{}", e))
        .unwrap();

    // We use a bounded channel to avoid the server using unreasonable
    // amounts of RAM if something goes wrong
    let (tx, rx) = mpsc::channel(250);
    {
        let config_ref = config.clone();
        std::thread::spawn(move || client_accept_thread(config_ref, tx));
    }

    server::server_main(rx, udp_listener).await;
}
