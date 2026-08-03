# Phase 1 — Terracotta 源码分析报告

> 目标：把 Terracotta 作为参考实现，分析其功能，并决定在 conic-terracotta 中
> 哪些逻辑需要迁移、哪些需要重写、哪些直接剔除。
> 本文所有协议细节都来自对 `Terracotta/src` 的逐文件阅读（crate 版本 `0.0.0-snapshot`，
> EasyTier fork `v2.5.0-terracotta.2`）。

---

## 1. 模块清单与职责

| 文件 | 行数 | 职责 | 是否迁移 |
|---|---|---|---|
| `main.rs` | 590 | 入口：CLI 参数、macOS daemon、HMCL 模式、Windows 控制台接管、单实例锁、日志重定向、EasyTier 生命周期 | 否（被 FFI 生命周期取代） |
| `lib.rs` | 360 | Android JNI 入口 | 否（目标平台是桌面 DLL） |
| `controller/mod.rs` | 14 | `SCAFFOLDING_PORT` 懒加载（起 scaffolding TCP server） | 是（改造） |
| `controller/api.rs` | 153 | `get_state / set_waiting / set_scanning / set_guesting` Web 入口 | 是（改造为命令/状态入口） |
| `controller/states.rs` | 196 | 全局 `AppState` 状态机 + 版本号（`index/sharing`）锁 | 是（改造为 session 状态） |
| `controller/rooms/mod.rs` | 31 | `Room`、`RoomKind`、`ConnectionDifficulty` | 是（原样） |
| `controller/rooms/scaffolding/mod.rs` | 39 | 持久化 `MACHINE_ID` / `VENDOR` | 是（原样） |
| `controller/rooms/scaffolding/room.rs` | 653 | **核心**：房间码生成/解析、host/guest 全流程、端口转发、MC 连通性检测 | 是（重点） |
| `controller/rooms/scaffolding/protocols.rs` | 116 | scaffolding 协议处理器（`c:ping` / `c:protocols` / `c:server_port` / `c:player_ping` / `c:player_profiles_list`） | 是（原样，协议兼容的关键） |
| `easytier/mod.rs` | 80 | EasyTier 门面 + `NatType` + `calc_conn_difficulty` | 是（原样） |
| `easytier/argument.rs` | 49 | `Argument` / `PortForward` / `Proto` → CLI 参数模型 | 是（原样） |
| `easytier/executable_impl.rs` | 325 | 桌面：解压嵌入的 `easytier-core`、起子进程、RPC CLI 调用、端口转发管理 | 是（原样） |
| `easytier/linkage_impl.rs` | 330 | Android：进程内链接 easytier crate | 否（桌面不用） |
| `easytier/publics.rs` | 14 | 公共节点列表 | 是（原样） |
| `mc/scanning.rs` | 157 | LAN 组播扫描 Minecraft 服务器 | 是（原样） |
| `mc/fakeserver.rs` | 61 | guest 端伪造 LAN 组播广播 | 是（原样） |
| `ports.rs` | 25 | 动态端口分配 | 是（原样） |
| `scaffolding/mod.rs` | 23 | `PacketResponse`、`TIMEOUT`(64s) | 是（原样） |
| `scaffolding/client.rs` | 159 | scaffolding TCP 客户端会话 | 是（原样） |
| `scaffolding/server.rs` | 89 | scaffolding TCP 服务器 + handler 注册 | 是（原样） |
| `scaffolding/profile.rs` | 49 | 玩家 Profile | 是（原样） |
| `server/mod.rs` + `states.rs` + `statics.rs` | ~272 | Rocket HTTP 服务器 + 嵌入 WebUI | 否（被 event queue + state query 取代） |
| `logging_windows.rs` / `lock_*` / `once_cell.rs` / `ui_macos.rs` | — | 平台工具 | 否 |
| `timestamp/` | — | `compile_time!` 宏（版本/升级检查） | 可选（conic 不需要） |

---

## 2. 必须复刻的协议与行为细节（兼容性关键）

以下细节决定"能否与原陶瓦用户联机"，必须逐字节兼容：

### 2.1 房间码（`room.rs:23-138`）
- 字符集：`0123456789ABCDEFGHJKLMNPQRSTUVWXYZ`（34 字符，去除易混淆 I/O）。
- 格式：`U/XXXX-XXXX-XXXX-XXXX`，16 位 base-34 数字（**低位在前**，`value % 34` 先输出）。
- 生成：`OsRng` 生成 u128，`% 34^16`，再向下取整到 7 的倍数（`value - value % 7`）。
- 解析：容忍任意前缀窗口扫描，校验分隔符与字符，并要求 `value % 7 == 0`。
- 派生：
  - `network_name = "scaffolding-mc-" + d0..d3 + "-" + d4..d7`
  - `network_secret = d8..d11 + "-" + d12..d15`
  - host 主机名：`scaffolding-mc-server-{scaffolding_port}`，host IP 固定 `10.144.144.1`。

### 2.2 EasyTier 启动参数（`room.rs:595-625`）
- 公共默认参数（顺序固定）：`--no-tun --compression=zstd --multi-thread --latency-first --enable-kcp-proxy -l udp://0.0.0.0:0 -l tcp://0.0.0.0:0 --p2p-only`。
- host 追加：`--hostname scaffolding-mc-server-{port} --ipv4 10.144.144.1 --tcp-whitelist {scaffolding_port} --tcp-whitelist {mc_port} --udp-whitelist {mc_port}`。
- guest 追加：`-d --tcp-whitelist 0 --udp-whitelist 0`。
- RPC 端口：`PortRequest::EasyTierRPC` 动态分配，CLI 用 `-p 127.0.0.1:{rpc}`。
- 公共节点（`publics.rs`）：
  `tcp://public.easytier.top:11010`、`tcp://public2.easytier.cn:54321`、
  `https://etnode.zkitefly.eu.org/node1`、`https://etnode.zkitefly.eu.org/node2`。

### 2.3 Scaffolding 线协议（`scaffolding/server.rs` + `client.rs`）
- 请求：`[1B kind_len][kind "ns:path"][4B body_len BE][body]`。
- 响应：`[1B status][4B body_len BE][body]`，status `0`=OK。
- 超时：`TIMEOUT = 64s`。
- handler：
  - `c:ping` — 回显；指纹校验常量 `[0x41,0x57,0x48,0x44,0x86,0x37,0x40,0x59,0x57,0x44,0x92,0x43,0x96,0x99,0x85,0x01]`。
  - `c:protocols` — 列出全部 `ns:path`，`\0` 分隔。
  - `c:server_port` — host 返回当前 MC 端口（2B BE）。
  - `c:player_ping` — guest 每 5s 上报 `{machine_id,name,vendor}`；host 刷新/添加 profile。
  - `c:player_profiles_list` — host 返回全部 profile 的 JSON 数组。
- server 端口：优先 `13448`，失败则 `0`（动态）。
- 客户端超时兜底：session 关闭判定 + `is_alive`。

### 2.4 Host 流程（`room.rs:140-216`）
1. 扫描 LAN（`mc/scanning.rs`：组播 `224.0.2.60:4445` / `FF75:230::60:4445`，过滤 MOTD）。
2. 取扫描到的第一个端口，起 EasyTier（host 参数），置 `HostOk`。
3. 后台监视线程每 5s：MC 连通性检测（`check_mc_conn`，发 `0xFE` 收 `0xFF`，3 次失败 → `PingServerRst`）；EasyTier 存活检查（→ `HostEasytierCrash`）；guest profile 超时 10s 剔除。

### 2.5 Guest 流程（`room.rs:218-593`）
1. 起 EasyTier（guest 参数），置 `GuestStarting`。
2. 轮询 peer 直到找到 host（hostname 前缀 `scaffolding-mc-server-`）→ 拿到 host IP + NAT。
3. 分配 scaffolding 本地端口，加 TCP 端口转发 `local→host:scaffolding_port`；计算连接难度。
4. 用指纹 `c:ping` 校验 scaffolding server。
5. 查询 `c:server_port` 得到 MC 端口；尝试申请**同号**本地端口（失败回退动态）；加 4 条转发
   （v4/v6 × TCP/UDP）`→ host_ip:mc_port`。
6. 验证 MC 连通性；建 FakeServer（组播广播 MOTD）+ 本地 profile，置 `GuestOk`。
7. 后台循环每 5s：`c:player_ping`、`c:player_profiles_list`，与本地 profile 集合 diff 并同步；EasyTier 存活检查。
8. 异常：`GuestEasytierCrash` / `PingHostFail` / `PingHostRst` / `PingServerRst` / `ScaffoldingInvalidResponse`。

### 2.6 状态机与版本（`states.rs`）
- `AppState::Waiting → HostScanning → HostStarting → HostOk` /
  `GuestConnecting → GuestStarting → GuestOk`，任意处可跳 `Exception`。
- `AppStateCapture`：流程持有进入时的版本，每次写状态 `index += 1`，`sharing` 用于只读刷新；
  流程在每个同步点检查 `capture.try_capture()`，若状态已被抢占则静默退出。
  这个"版本令牌"语义是我们 event/state 设计的基础。

### 2.7 其它
- 端口分配（`ports.rs`）：绑定 `127.0.0.1:0` 取端口，失败回退 `35780 + 变体号`。
- `MACHINE_ID`：16 随机字节 hex，持久化到 `<data_root>/machine-id`。
- `MOTD`：`§6§l双击进入陶瓦联机大厅（请保持陶瓦运行）`（guest 端 FakeServer 广播内容，host 扫描要过滤掉）。
- `build.rs`：从 `burningtnt/EasyTier` releases 下载 `v2.5.0-terracotta.2` 平台包，
  LZMA2 7z 重打包后 `include_bytes!` 嵌入。

---

## 3. 结论：需要迁移 / 重写 / 剔除

**原样迁移（协议与行为耦合，直接移植）：**
- `room.rs` 的房间码生成/解析（`create_room` / `parse` / `from_value` / 字符表）
- `protocols.rs` 全部 handler（`HANDLERS` 表）
- `scaffolding/server.rs`、`client.rs`、`profile.rs`、`mod.rs`
- `easytier/argument.rs`、`executable_impl.rs`、`publics.rs`、`mod.rs`（桌面版）
- `mc/scanning.rs`、`fakeserver.rs`、`ports.rs`
- `machine-id` 持久化逻辑

**改造重写（保留行为，重组织架构）：**
- `states.rs` 的全局 `AppState` → per-context `Session` 状态机（去掉 150ms 锁检测，改用队列语义）
- `api.rs` 的 Web 入口 → 命令入口（`Command` enum）
- `room.rs` 的 host/guest 流程 → 在 tokio runtime 内以 `spawn_blocking` 运行，同步点改用版本令牌 + 事件推送
- `main.rs` 的 EasyTier 生命周期 → 由 context `drop`/destroy 管理

**剔除（新架构不需要）：**
- `server/`（Rocket HTTP + WebUI）、`lib.rs`（Android JNI）、`main.rs`（CLI/daemon/HMCL）、
  `lock_*`、`logging_windows`、`ui_macos`、`once_cell.rs`、`linkage_impl.rs`、`timestamp`。

---

## 4. 推荐 conic-terracotta 项目结构

```
conic-terracotta/
├── Cargo.toml            # cdylib，edition 2024，stable Rust（不依赖 nightly）
├── build.rs              # 移植 Terracotta 的 EasyTier 下载/7z 重打包/嵌入
├── include/terracotta.h  # 对外 C ABI
├── docs/
└── src/
    ├── lib.rs            # crate 根：模块声明、panic 边界、FFI 导出
    ├── ffi.rs            # C ABI 层（handle / 字符串 / 错误码转换）
    ├── context.rs        # TerracottaContext：runtime thread + 双队列 + session
    ├── command.rs        # Command enum（FFI → runtime）
    ├── event.rs          # Event enum + JSON payload（runtime → FFI）
    ├── session.rs        # Session 状态机（Waiting/Host*/Guest*/Exception）+ 版本令牌
    ├── flow.rs           # host/guest 流程（spawn_blocking 内运行）
    ├── room.rs           # 房间码协议（迁移）
    ├── ports.rs          # 端口分配（迁移）
    ├── machine_id.rs     # 持久化 machine id（迁移）
    ├── easytier/
    │   ├── mod.rs        # 门面 + NatType + calc_conn_difficulty（迁移）
    │   ├── args.rs       # Argument/PortForward/Proto（迁移）
    │   ├── process.rs    # 子进程 + RPC CLI + 端口转发（迁移自 executable_impl.rs）
    │   └── public_nodes.rs
    ├── scaffolding/
    │   ├── mod.rs        # PacketResponse、TIMEOUT（迁移）
    │   ├── server.rs     # TCP server + handler 注册（迁移）
    │   ├── client.rs     # TCP client 会话（迁移）
    │   ├── profile.rs    # Profile（迁移）
    │   └── protocols.rs  # HANDLERS（迁移）
    └── mc/
        ├── scanner.rs    # LAN 组播扫描（迁移）
        └── fakeserver.rs # 组播广播（迁移）
```
