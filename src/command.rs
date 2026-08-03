#[derive(Debug)]
pub enum Command {
    CreateRoom {
        player_name: Option<String>,
        room_code: Option<String>,
        public_nodes: Vec<String>,
    },
    JoinRoom {
        room_code: String,
        player_name: Option<String>,
        public_nodes: Vec<String>,
    },
    SetWaiting,
    Shutdown,
}
