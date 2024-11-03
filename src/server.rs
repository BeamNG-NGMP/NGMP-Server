use std::collections::HashMap;
use std::net::SocketAddr;

use tokio::sync::mpsc;

use serde::{Deserialize, Serialize};

use ngmp_protocol_impl::server_launcher::gameplay::{VehicleData, VehicleUpdatePacket};
use ngmp_protocol_impl::server_launcher::Packet;
use ngmp_protocol_impl::{connection::*, server_launcher};

use crate::{http::User, plugin::LuaEnvironment};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct VehicleTransformData {
    // m
    pos: [f32; 3],
    // help
    rot: [f32; 4],
    // m/s
    vel: [f32; 3],
    // rad/s
    rvel: [f32; 3],
    // ms since client connection started
    ms: u32,
}

pub struct Vehicle {
    veh_data: VehicleData,

    latest_transform: VehicleTransformData,
    latest_runtime: VehicleUpdatePacket,
}

impl Vehicle {
    pub fn new(veh_data: VehicleData) -> Self {
        Self {
            veh_data,
            latest_transform: VehicleTransformData::default(),
            latest_runtime: VehicleUpdatePacket::default(),
        }
    }
}

pub struct Client {
    pub tcp_conn: TcpConnection<Packet>,
    pub udp_addr: SocketAddr,

    pub steam_id: u64,
    pub user: User,

    pub synced: bool,

    pub vehicles: HashMap<u16, Vehicle>,
}

impl Client {
    pub fn new(
        tcp_conn: TcpConnection<Packet>,
        udp_addr: SocketAddr,
        steam_id: u64,
        user: User,
    ) -> Self {
        Self {
            tcp_conn,
            udp_addr,

            steam_id,
            user,

            synced: false,

            vehicles: HashMap::new(),
        }
    }

    async fn tcp_try_recv(&mut self) -> anyhow::Result<Option<Packet>> {
        self.tcp_conn.try_read_packet().await
    }

    /// This function returns None if it failed to create a new vehicle ID
    fn add_vehicle(&mut self, veh_data: VehicleData) -> Option<u16> {
        for i in 0..u16::MAX {
            if self.vehicles.get(&i).is_none() {
                self.vehicles.insert(i, Vehicle::new(veh_data));
                return Some(i);
            }
        }
        None
    }
}

struct ServerClients(HashMap<u64, Client>);

impl ServerClients {
    fn get_client_from_udp_addr(&self, addr: SocketAddr) -> Option<&Client> {
        for (_, client) in &self.0 {
            if client.udp_addr == addr {
                return Some(client);
            }
        }
        None
    }

    fn get_mut_client_from_udp_addr(&mut self, addr: SocketAddr) -> Option<&mut Client> {
        for (_, client) in &mut self.0 {
            if client.udp_addr == addr {
                return Some(client);
            }
        }
        None
    }

    async fn tcp_gather_packets(&mut self) -> Vec<(u64, Packet)> {
        let mut packets = Vec::new();

        let mut to_remove = Vec::new();

        for (id, client) in &mut self.0 {
            match client.tcp_try_recv().await {
                Ok(maybe_packet) => {
                    if let Some(packet) = maybe_packet {
                        packets.push((*id, packet));
                    }
                }
                Err(e) => {
                    error!("{}", e);
                    to_remove.push(*id);
                }
            }
        }

        for id in to_remove {
            self.0.remove(&id);
        }

        packets
    }

    async fn tcp_broadcast_packet(&mut self, packet: Packet, exclude_id: Option<u64>) {
        trace!("Broadcasting packet: {:?}", packet);
        let mut to_remove = Vec::new();

        for (id, client) in &mut self.0 {
            if Some(*id) == exclude_id {
                continue;
            }
            if let Err(e) = client.tcp_conn.write_packet(&packet).await {
                error!("{}", e);
                to_remove.push(*id);
            }
        }

        for id in to_remove {
            self.0.remove(&id);
        }
    }
}

struct ServerUdp(UdpListener<Packet>);

impl ServerUdp {
    async fn udp_gather_packets(&mut self) -> Vec<(SocketAddr, Packet)> {
        let mut packets = Vec::new();

        loop {
            match self.0.try_read_packet() {
                Ok(Some((packet, addr))) => packets.push((addr, packet)),
                Ok(None) => break, // Done reading packets!
                Err(e) => {
                    error!("{}", e);
                }
            }
        }

        packets
    }

    async fn udp_send_packet(&mut self, addr: SocketAddr, packet: Packet) -> anyhow::Result<()> {
        self.0.write_packet(addr, packet).await
    }
}

struct Server {
    udp: ServerUdp,
    clients: ServerClients,

    plugins: LuaEnvironment,

    update_player_data_flag: bool,
}

impl Server {
    fn new(udp_socket: UdpListener<Packet>) -> Self {
        Self {
            udp: ServerUdp(udp_socket),
            clients: ServerClients(HashMap::new()),

            // TODO: Error handling here please :3
            plugins: LuaEnvironment::new().expect("Failed to load Lua plugin system!"),

            update_player_data_flag: false,
        }
    }

    async fn tick(&mut self) {
        let tcp_packets = self.clients.tcp_gather_packets().await;
        let udp_packets = self.udp.udp_gather_packets().await;

        for (steam_id, packet) in tcp_packets {
            self.tcp_handle_packet(steam_id, packet).await;
        }

        for (udp_addr, packet) in udp_packets {
            self.udp_handle_packet(udp_addr, packet).await;
        }

        // Update all vehicle positions and runtime data
        for (steam_id, client) in self.clients.0.iter() {
            for (veh_id, veh) in client.vehicles.iter() {
                for (s2, c2) in self.clients.0.iter() {
                    if s2 == steam_id {
                        continue;
                    }

                    // Position packet
                    if veh.latest_transform.ms > 0 {
                        if let Err(e) = self
                            .udp
                            .udp_send_packet(
                                c2.udp_addr,
                                Packet::VehicleTransform(
                                    server_launcher::gameplay::VehicleTransformPacket {
                                        player_id: *steam_id,
                                        vehicle_id: *veh_id,
                                        transform: serde_json::to_string(&veh.latest_transform)
                                            .expect("Somehow failed to serialize to json!"),
                                    },
                                ),
                            )
                            .await
                        {
                            error!("{}", e);
                        }
                    }

                    // Runtime data
                    if veh.latest_runtime.ms > 0 {
                        if let Err(e) = self
                            .udp
                            .udp_send_packet(
                                c2.udp_addr,
                                Packet::VehicleUpdate(veh.latest_runtime.clone()),
                            )
                            .await
                        {
                            error!("{}", e);
                        }
                    }
                }
            }
        }

        if self.update_player_data_flag {
            trace!("Update player data flag is true.");
            self.update_player_data_flag = false;

            let players = self
                .clients
                .0
                .iter()
                .map(|(id, client)| server_launcher::gameplay::PlayerData {
                    name: client.user.name.clone(),
                    steam_id: *id,
                    avatar_hash: client.user.avatar_hash.clone(),
                })
                .collect::<Vec<_>>();
            self.clients
                .tcp_broadcast_packet(
                    Packet::PlayerData(server_launcher::gameplay::PlayerDataPacket { players }),
                    None,
                )
                .await;
        }
    }

    async fn tcp_handle_packet(&mut self, steam_id: u64, packet: Packet) {
        match packet {
            Packet::VehicleSpawn(mut p) => {
                let mut block_spawn = false;
                if let Some(veh_id) = self.spawn_vehicle(steam_id, p.vehicle_data.clone()).await {
                    if let Some(client) = self.clients.0.get_mut(&steam_id) {
                        if let Err(e) = client
                            .tcp_conn
                            .write_packet(&Packet::VehicleConfirm(
                                server_launcher::gameplay::VehicleConfirmPacket {
                                    confirm_id: p.confirm_id,
                                    vehicle_id: veh_id,
                                    obj_id: p.vehicle_data.object_id,
                                },
                            ))
                            .await
                        {
                            error!("{}", e);
                            block_spawn = true;
                            // TODO: Kick client because error
                        }
                    } else {
                        block_spawn = true;
                    }
                    if !block_spawn {
                        trace!("spawning vehicle ({veh_id})");
                        p.vehicle_id = veh_id;
                        self.clients
                            .tcp_broadcast_packet(Packet::VehicleSpawn(p), Some(steam_id))
                            .await;
                    }
                } else {
                    block_spawn = true;
                }
                if block_spawn {
                    error!("block spawn?");
                    todo!();
                }
            }
            _ => error!("Unsupported packet (TCP): {:?}", packet),
        }
    }

    async fn udp_handle_packet(&mut self, addr: SocketAddr, packet: Packet) {
        let player_id = if let Some(client) = self.clients.get_client_from_udp_addr(addr) {
            client.steam_id
        } else {
            return;
        };

        match packet {
            Packet::VehicleTransform(p) => {
                // You can only affect your own vehicle!
                if p.player_id == player_id {
                    if let Ok(parsed) = serde_json::from_str::<VehicleTransformData>(&p.transform) {
                        let client = self.clients.get_mut_client_from_udp_addr(addr).unwrap();
                        if let Some(veh) = client.vehicles.get(&p.vehicle_id) {
                            if parsed.ms > veh.latest_transform.ms {
                                let veh = client.vehicles.get_mut(&p.vehicle_id).unwrap();
                                veh.latest_transform = parsed;
                            }
                        }
                    } else {
                        error!("Failed to parse vehicle transform data!");
                    }
                }
            }
            Packet::VehicleUpdate(p) => {
                if p.player_id == player_id {
                    let client = self.clients.get_mut_client_from_udp_addr(addr).unwrap();
                    if let Some(veh) = client.vehicles.get(&p.vehicle_id) {
                        if p.ms > veh.latest_runtime.ms {
                            let veh = client.vehicles.get_mut(&p.vehicle_id).unwrap();
                            veh.latest_runtime = p;
                        }
                    }
                }
            }
            _ => error!("Unsupported packet (UDP): {:?} ({})", packet, addr),
        }
    }

    async fn add_client(&mut self, client: Client) {
        trace!("Client arrived at server");
        self.update_player_data_flag = true;

        let steam_id = client.steam_id.clone();
        let name = client.user.name.clone();

        self.clients.0.insert(client.steam_id, client);

        self.plugins.event_on_player_auth(steam_id, name).await;
    }

    /// Returns None if it failed to spawn a vehicle
    async fn spawn_vehicle(&mut self, steam_id: u64, veh_data: VehicleData) -> Option<u16> {
        if let Some(client) = self.clients.0.get_mut(&steam_id) {
            return client.add_vehicle(veh_data);
        }
        None
    }
}

pub async fn server_main(mut rx: mpsc::Receiver<Client>, udp_listener: UdpListener<Packet>) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(20)); // 20ms = 50 ticks per second
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    let mut server = Server::new(udp_listener);
    info!("Server running!");

    // Load plugins
    // TODO: Look inside plugins folder to discover plugins
    if let Err(e) = server
        .plugins
        .load_plugin("broken".to_string(), "plugins/broken/main.lua")
        .await
    {
        error!("uh oh {}", e);
    }
    if let Err(e) = server
        .plugins
        .load_plugin("example".to_string(), "plugins/example/main.lua")
        .await
    {
        error!("uh oh {}", e);
    }

    loop {
        match rx.try_recv() {
            Ok(client) => {
                server.add_client(client).await;
            }
            Err(mpsc::error::TryRecvError::Empty) => {} // Ignore and continue
            Err(_) => {
                error!("Connection to client accept thread lost! Closing server, this is unrecoverable...");
                break;
            }
        }

        // TODO: Measure ticks per second of this loop to make sure we are running
        //       at roughly 50tps
        let mut need_tick = true;
        tokio::select!(
            _ = server.tick() => {},
            _ = interval.tick() => {
                trace!("INTERVAL CANCELLED SERVER TICK");
                need_tick = false;
            },
        );
        if need_tick {
            interval.tick().await;
        }
    }
}
