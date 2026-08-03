use crate::easytier::argument::{Argument, PortForward};
use crate::easytier::{EasyTierMember, NatType};
use crate::ports::PortRequest;
use parking_lot::Mutex;
use std::ffi::OsString;
use std::fmt::Write;
use std::io::{self, BufRead, BufReader, Cursor, Error};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::str::FromStr;
use std::sync::{mpsc, Arc, OnceLock};
use std::thread;
use std::time::Duration;
use std::{env, fs};

static EASYTIER_ARCHIVE: (&str, &str, &[u8]) = (
    include_str!(env!("TERRACOTTA_ET_ENTRY_CONF")),
    include_str!(env!("TERRACOTTA_ET_CLI_CONF")),
    include_bytes!(env!("TERRACOTTA_ET_ARCHIVE")),
);

static FACTORY: OnceLock<Mutex<Option<Arc<EasytierFactory>>>> = OnceLock::new();

pub(crate) struct EasytierFactory {
    exe: PathBuf,
    cli: PathBuf,
    data_dir: PathBuf,
}

pub(crate) fn initialize(data_dir: &Path) -> std::io::Result<Arc<EasytierFactory>> {
    let lock = FACTORY.get_or_init(|| Mutex::new(None));

    let mut factory = lock.lock();
    if let Some(f) = factory.as_ref().filter(|f| f.data_dir == data_dir) {
        return Ok(f.clone());
    }

    let built = Arc::new(create_factory(data_dir)?);
    *factory = Some(built.clone());
    Ok(built)
}

fn create_factory(data_dir: &Path) -> io::Result<EasytierFactory> {
    let dir = data_dir.join("embedded-easytier");
    let _ = fs::create_dir_all(&dir);

    logging!(
        "Easytier",
        "Releasing easytier to {}",
        dir.to_string_lossy()
    );

    sevenz_rust2::decompress(Cursor::new(EASYTIER_ARCHIVE.2.to_vec()), &dir)
        .map_err(|e| Error::other(e.to_string()))?;

    let exe = dir.join(EASYTIER_ARCHIVE.0);
    let cli = dir.join(EASYTIER_ARCHIVE.1);
    #[cfg(target_family = "unix")]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(exe.clone())?.permissions();
        permissions.set_mode(permissions.mode() | 0o100);
        fs::set_permissions(exe.clone(), permissions)?;

        let mut permissions = fs::metadata(cli.clone())?.permissions();
        permissions.set_mode(permissions.mode() | 0o100);
        fs::set_permissions(cli.clone(), permissions)?;
    }
    Ok(EasytierFactory { exe, cli, data_dir: data_dir.to_path_buf() })
}

pub struct EasyTier {
    process: Arc<Mutex<Child>>,
    rpc: u16,
    factory: Arc<EasytierFactory>,
}

pub fn create(data_dir: &Path, args: Vec<Argument>) -> EasyTier {
    let factory = match initialize(data_dir) {
        Ok(factory) => factory,
        Err(e) => panic!("easytier factory for {}: {e}", data_dir.display()),
    };

    let args = {
        let mut built: Vec<OsString> = Vec::with_capacity((args.len() as f32 * 1.5).floor() as usize);

        macro_rules! push {
            ($($item:expr),* $(,)?) => {
                built.extend_from_slice(&[$($item.into()),*])
            };
        }
        for arg in args {
            match arg {
                Argument::NoTun => push!["--no-tun"],
                Argument::Compression(method) => push![format!("--compression={}", method)],
                Argument::MultiThread => push!["--multi-thread"],
                Argument::LatencyFirst => push!["--latency-first"],
                Argument::EnableKcpProxy => push!["--enable-kcp-proxy"],
                Argument::PublicServer(server) => push!["-p", server.as_ref()],
                Argument::NetworkName(name) => push!["--network-name", name.as_ref()],
                Argument::NetworkSecret(secret) => push!["--network-secret", secret.as_ref()],
                Argument::Listener { address, proto } => push!["-l", format!("{}://{}", proto.name(), address)],
                Argument::PortForward(PortForward { local, remote, proto }) => push![
                    format!("--port-forward={}://{}/{}", proto.name(), local, remote)
                ],
                Argument::Dhcp => push!["-d"],
                Argument::HostName(name) => push!["--hostname", name.as_ref()],
                Argument::IPv4(address) => push!["--ipv4", address.to_string()],
                Argument::TcpWhitelist(port) => push![format!("--tcp-whitelist={}", port)],
                Argument::UdpWhitelist(port) => push![format!("--udp-whitelist={}", port)],
                Argument::P2POnly => push!["--p2p-only"],
            }
        }
        built
    };

    fs::metadata(&factory.exe).unwrap();

    let rpc = PortRequest::EasyTierRPC.request();

    logging!("Easytier", "Starting easytier: {:?}, rpc={}", args, rpc);

    let mut process = Command::new(&factory.exe);
    process
        .args(args)
        .args(["-r", &rpc.to_string()])
        .current_dir(env::temp_dir())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_family = "windows")]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x08000000);
    }

    let mut process = process.spawn().unwrap();

    let (sender, receiver) = mpsc::channel::<String>();
    forward_std(&mut process, move |line| {
        let _ = sender.send(line);
    });

    let process: Arc<Mutex<Child>> = Arc::new(Mutex::new(process));
    let process2 = process.clone();

    thread::spawn(move || {
        const LINES: usize = 500;

        let mut buffer: [Option<String>; LINES] = [const { None }; LINES];
        let mut index = 0;

        let status = 'status: loop {
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(value) => {
                    buffer[index] = Some(value);
                    index = (index + 1) % LINES;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    match process2.lock().try_wait() {
                        Ok(Some(status)) => break 'status Some(status),
                        Ok(None) => {
                            logging!("EasyTier", "Cannot fetch EasyTier process status: EasyTier hasn't exited.");
                        }
                        Err(e) => {
                            logging!("EasyTier", "Cannot fetch EasyTier process status: {:?}", e);
                        }
                    }
                    break 'status None;
                }
            }
        };

        let mut output = String::from("Easytier has exited. with status ");
        match status {
            Some(status) => match status.code() {
                Some(code) => write!(output, "code={}, success={}", code, status.success()),
                None => write!(output, "code=[unknown], success={}", status.success()),
            }
            .unwrap(),
            None => output.push_str("[unknown]"),
        }
        output.push_str(". Here's the logs:\n############################################################");
        for i in 0..LINES {
            if let Some(value) = &buffer[(index + 1 + i) % LINES] {
                output.push_str("\n    ");
                output.push_str(value);
            }
        }
        output.push_str("\n############################################################");

        logging!("Easytier", "{}", output);
    });

    EasyTier { process, rpc, factory }
}

fn forward_std<F>(process: &mut Child, handle: F)
where
    F: Fn(String) + Send + Sized + Clone + 'static,
{
    let handle2 = handle.clone();

    let stdout = process.stdout.take().unwrap();
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            handle(line);
        }
    });

    let stderr = process.stderr.take().unwrap();
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            handle2(line);
        }
    });
}

impl EasyTier {
    pub fn is_alive(&self) -> bool {
        matches!(self.process.lock().try_wait(), Ok(None))
    }

    pub fn get_players(&self) -> Option<Vec<EasyTierMember>> {
        let object: serde_json::Value = serde_json::from_str(std::str::from_utf8(
            &self.start_cli()
                .args(["-p", &format!("127.0.0.1:{}", self.rpc), "-o", "json", "peer"])
                .output()
                .ok()?
                .stdout,
        )
        .ok()?)
        .ok()?;

        let mut players: Vec<EasyTierMember> = vec![];
        for item in object.as_array()? {
            let hostname = item.as_object()?.get("hostname")?.as_str()?.to_string();
            let address = Ipv4Addr::from_str(item.as_object()?.get("ipv4")?.as_str()?).ok();
            let is_local = item.as_object()?.get("cost")?.as_str()? == "Local";
            let nat = match item.as_object()?.get("nat_type")?.as_str()? {
                "Unknown" => NatType::Unknown,
                "OpenInternet" => NatType::OpenInternet,
                "NoPat" => NatType::NoPAT,
                "FullCone" => NatType::FullCone,
                "Restricted" => NatType::Restricted,
                "PortRestricted" => NatType::PortRestricted,
                "Symmetric" => NatType::Symmetric,
                "SymUdpFirewall" => NatType::SymmetricUdpWall,
                "SymmetricEasyInc" => NatType::SymmetricEasyIncrease,
                "SymmetricEasyDec" => NatType::SymmetricEasyDecrease,
                #[cfg(debug_assertions)]
                nat => panic!("Unknown NAT type: {}", nat),
                #[cfg(not(debug_assertions))]
                _ => return None,
            };

            players.push(EasyTierMember { hostname, address, is_local, nat });
        }
        Some(players)
    }

    pub fn add_port_forward(&mut self, forwards: &[PortForward]) -> bool {
        let mut processes: Vec<(&PortForward, Option<Child>)> =
            forwards.iter().map(|forward| (forward, None)).collect();

        for time in 0..3 {
            for (PortForward { local, remote, proto }, process_holder) in processes.iter_mut() {
                let mut process = match self.start_cli().args([
                    "-p", &format!("127.0.0.1:{}", self.rpc), "port-forward", "add",
                    proto.name(), &local.to_string(), &remote.to_string(),
                ]).spawn() {
                    Ok(v) => v,
                    Err(e) => {
                        logging!("EasyTier CLI", "Cannot spawn easytier cli instance: {:?}", e);
                        return false;
                    }
                };
                forward_std(&mut process, |line| {
                    logging!("EasyTier CLI", "{}", line);
                });

                process_holder.replace(process);
            }

            for i in (0..processes.len()).rev() {
                if processes[i].1.as_mut().unwrap().wait().is_ok_and(|status| status.success()) {
                    processes.swap_remove(i);
                }
            }

            if processes.is_empty() {
                return true;
            }

            thread::sleep(Duration::from_millis(time * 1000 + 500))
        }

        if !processes.is_empty() {
            let mut msg = "Cannot adding port-forward rules: ".to_string();
            for (i, (PortForward { local, remote, proto }, _)) in processes.iter().enumerate() {
                write!(&mut msg, "{} -> {} ({})", local, remote, proto.name()).unwrap();
                if i != processes.len() - 1 {
                    msg.push_str(", ");
                }
            }
            logging!("EasyTier CLI", "{}", msg);
            return false;
        }
        true
    }

    fn start_cli(&self) -> Command {
        let mut command = Command::new(&self.factory.cli);
        command
            .current_dir(env::temp_dir())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        command
    }
}

impl Drop for EasyTier {
    fn drop(&mut self) {
        logging!("EasyTier", "Killing EasyTier.");
        let _ = self.process.lock().kill();
    }
}

#[cfg(test)]
mod tests {
    use crate::easytier::argument::{Argument, PortForward, Proto};
    use crate::easytier::{create, initialize};
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
    use std::time::Duration;

    #[test]
    fn spawns_and_reports_health() {
        let data_dir = std::env::temp_dir().join(format!(
            "conic-terracotta-easytier-test-{}",
            std::process::id()
        ));
        initialize(&data_dir).unwrap();

        let mut easytier = create(
            &data_dir,
            vec![
                Argument::NoTun,
                Argument::NetworkName("conic-terracotta-test".into()),
                Argument::NetworkSecret("test-test".into()),
                Argument::P2POnly,
            ],
        );

        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        let mut alive = false;
        while std::time::Instant::now() < deadline {
            if easytier.is_alive() {
                alive = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(alive, "easytier-core process must stay alive");
        assert!(easytier.is_alive());

        let mut health = false;
        while std::time::Instant::now() < deadline {
            if let Some(players) = easytier.get_players() {
                let _ = players;
                health = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(health, "easytier RPC must answer `peer`");

        assert!(easytier.add_port_forward(&[PortForward {
            local: SocketAddrV4::new(Ipv4Addr::LOCALHOST, 25566).into(),
            remote: SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 25566, 0, 0).into(),
            proto: Proto::Tcp,
        }]));
    }
}
