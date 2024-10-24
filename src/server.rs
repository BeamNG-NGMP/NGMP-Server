use std::net::SocketAddr;

use tokio::sync::mpsc;

use ngmp_protocol_impl::{
    connection::*,
    server_launcher,
};
use ngmp_protocol_impl::server_launcher::Packet;

pub struct Client {
    pub tcp_conn: TcpConnection<Packet>,
    pub udp_addr: SocketAddr,

    // TODO: Add "synced" flag
}

impl Client {
    async fn tcp_try_recv(&mut self) -> anyhow::Result<Option<Packet>> {
        self.tcp_conn.try_read_packet().await
    }
}

struct ServerClients(Vec<Client>);

impl ServerClients {
    async fn tcp_gather_packets(&mut self) -> Vec<Packet> {
        let mut packets = Vec::new();

        for client in &mut self.0 {
            match client.tcp_try_recv().await {
                Ok(maybe_packet) => if let Some(packet) = maybe_packet {
                    packets.push(packet);
                },
                Err(e) => {
                    error!("{}", e);
                    todo!();
                },
            }
        }

        packets
    }
}

struct ServerUdp(UdpListener<Packet>);

impl ServerUdp {
    async fn udp_gather_packets(&mut self) -> Vec<Packet> {
        let mut packets = Vec::new();

        packets
    }
}

struct Server {
    udp: ServerUdp,

    clients: ServerClients,
}

impl Server {
    fn new(udp_socket: UdpListener<Packet>) -> Self {
        Self {
            udp: ServerUdp(udp_socket),
            clients: ServerClients(Vec::new()),
        }
    }

    async fn tick(&mut self) {
        let tcp_packets = self.clients.tcp_gather_packets().await;
        // let udp_packets = self.udp.udp_gather_packets().await;

        if tcp_packets.len() > 0 {
            debug!("tcp_packets: {:?}", tcp_packets);
        }
    }

    async fn add_client(&mut self, client: Client) {
        self.clients.0.push(client);
    }
}

pub async fn server_main(mut rx: mpsc::Receiver<Client>, udp_listener: UdpListener<Packet>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(20)); // 20ms = 50 ticks per second
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut server = Server::new(udp_listener);

    info!("Server running!");

    loop {
        match rx.try_recv() {
            Ok(client) => {
                server.add_client(client).await;
            },
            Err(mpsc::error::TryRecvError::Empty) => {}, // Ignore and continue
            Err(_) => {
                error!("Connection to client accept thread lost! Closing server, this is unrecoverable...");
                break;
            }
        }

        // TODO: Measure ticks per second of this loop to make sure we are running
        //       at roughly 50tps
        tokio::select!(
            _ = server.tick() => {},
            _ = interval.tick() => {},
        );
    }
}
