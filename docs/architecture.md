# Phase 2 — conic-terracotta 内部架构设计

> 目标：一个**独立的动态库**，内部自建 tokio runtime 与后台线程，对外只暴露
> 稳定的 C ABI。FFI 调用线程永远不直接执行复杂异步逻辑 —— 一切通过
> command queue 派发到 runtime 线程，状态通过 event queue（增量）+ state query（全量）暴露。

---

## 1. 总体结构

```
                    ┌────────────────── terracotta.dll ───────────────────┐
Conic (C ABI)       │                                                      │
┌───────────────┐   │   ┌───────────────────────────────┐                  │
│ terracotta_   │───┼──►│      command queue            │  (tokio::sync)   │
│ create_room   │   │   │  UnboundedSender<Command>     │                  │
│ join_room     │   │   └──────────────┬────────────────┘                  │
│ set_waiting   │   │                  │                                   │
│ get_state     │◄──┼── state snapshot │                                   │
│ poll_event    │◄──┼── event (std::mpsc, non-blocking)                    │
└───────────────┘   │                  │                                   │
                    │                  ▼                                   │
                    │   ┌──────────────────────────────────────────┐      │
                    │   │      runtime thread (tokio runtime)      │      │
                    │   │   block_on( main_loop(ctx) )             │      │
                    │   │   - 消费 Command（异步 select）           │      │
                    │   │   - 用 spawn_blocking 派发 host/guest 流程│      │
                    │   │   - 生命周期 / 优雅关闭                    │      │
                    │   └───────┬───────────────────────┬──────────┘      │
                    │           │ spawn_blocking        │ spawn_blocking  │
                    │           ▼                       ▼                 │
                    │   ┌──────────────────┐   ┌──────────────────┐       │
                    │   │ host/guest flow  │   │ 后台守护任务       │       │
                    │   │ (room.rs 移植)   │   │ (监视线程)         │       │
                    │   └───────┬──────────┘   └────────┬─────────┘       │
                    │           │                       │                 │
                    │           ▼                       ▼                 │
                    │   ┌────────────────────────────────────┐            │
                    │   │  Session (Arc<parking_lot::Mutex>) │            │
                    │   │  状态机 + 版本令牌 + profiles      │            │
                    │   └───────┬───────────────┬───────────┘            │
                    │           │               │                         │
                    │   easytier subprocess   scaffolding server/         │
                    │   (spawn/CLI/RPC)        mc scanner & fakeserver    │
                    │                                                      │
                    │   event queue: std::sync::mpsc::Sender<Event>        │
                    └──────────────────────────────────────────────────────┘
```

---

## 2. TerracottaContext

```rust
pub struct TerracottaContext {
    runtime: Runtime,                        // 自有 tokio multi-thread runtime
    command_tx: UnboundedSender<Command>,    // FFI → runtime thread
    events_tx: std::sync::mpsc::Sender<Event>,  // 内部 → FFI 事件队列
    events_rx: std::sync::mpsc::Receiver<Event>, // FFI poll_event 消费
    session: Arc<Mutex<Session>>,            // 完整状态快照（全量查询）
    shutdown: Arc<AtomicBool>,               // 优雅关闭信号
    config: Mutex<TerracottaConfig>,         // public nodes / data dir
    runtime_thread: Option<JoinHandle<()>>,  // 后台 runtime 线程
    next_seq: AtomicU64,                     // 事件序号（FFI 侧断线检测）
}
```

**关键点**
- runtime 线程 = 一个 std::thread，`runtime.block_on(main_loop(ctx))`。runtime 随
  context 存活；destroy 时先发 `Command::Shutdown`，join 线程，再 drop runtime。
- `main_loop` 是一个 async 任务：
  - `tokio::select!` 监听 command_rx 与 shutdown；
  - 收到 `CreateRoom / JoinRoom / SetWaiting` 后调用 `session` 校验前置状态，
    `spawn_blocking` 启动对应流程（流程持有版本令牌）；
  - 收到 `Shutdown` 置 shutdown 标志并返回。
- FFI 线程只做三件事：`try_send`（命令）、`lock + 拷贝`（state）、`try_recv`（event）。

### 为什么不直接用 callback / 共享对象
- callback 在 DLL 卸载、线程退出、对象生命周期上容易出竞态；
- 事件 + 状态查询：增量更新走 event，断线恢复走 state，二者互补，且天然可重入。

---

## 3. Command（命令队列）

```rust
pub enum Command {
    CreateRoom {
        player_name: Option<String>,
        room_code: Option<String>,   // None → 新房间
        public_nodes: Vec<String>,   // 合并配置与内置节点
    },
    JoinRoom {
        room_code: String,
        player_name: Option<String>,
        public_nodes: Vec<String>,
    },
    SetWaiting,       // 强制回到 Waiting（版本号自增 → 使流程令牌失效）
    Shutdown,
}
```

**两层级队列入队策略**（用户需求）：
- **命令队列（不可丢失）**：`tokio::sync::mpsc::UnboundedSender<Command>`。
  create/join/leave 等用户操作必须可靠送达 runtime 线程，任何情况下不允许丢弃。
- **事件队列（可丢弃）**：`std::sync::mpsc::sync_channel(256)` + `try_send`。
  事件只是增量提示，满了就丢并累计 `dropped` 计数；Conic 可通过
  `terracotta_get_state()` 全量恢复，因此丢失事件不会破坏一致性。

命令在 FFI 层做**同步前置校验**（如房间码合法、当前状态允许该命令），
校验通过后才入队 —— 这样 `terracotta_create_room` 可以立即返回可解释的错误码，
而不需要等 runtime 线程处理。

---

## 4. Event（事件队列）

```rust
pub enum Event {
    StateChanged { state: SessionStateId, version: u64 },
    PlayerJoined { machine_id: String, name: String, vendor: String, kind: u8 },
    PlayerLeft  { machine_id: String },
    ConnectionDifficulty { difficulty: u8 },
    HostReady   { room_code: String, port: u16 },   // host 端 MC 端口已就绪
    GuestReady  { url: String },                    // "127.0.0.1:port"（guest 端连这个）
    Error       { code: u32, message: String },
}
```

- 每个事件带全局递增 `sequence`（FFI 侧可检测丢事件）。
- payload 在 FFI 层统一序列化为 **JSON 字符串**，减少 C 结构体版本漂移。
- 事件由三类生产者写入：
  1. `main_loop`（状态转换）；
  2. host/guest 流程（阶段推进、玩家进出、错误）；
  3. scaffolding server 的 handler（`c:player_ping` → `PlayerJoined/Left`）。
- 所有事件产出都发生在 `session` 锁**已释放之后**（先改状态，再发事件）。

### 版本令牌（对应原 `AppStateCapture`）
- `Session.version` 单调递增。流程开始时记下 `token = version`。
- 每次流程要"提交"下一步时：`session.try_capture(token)` —— 若当前 version 已变
  （被 SetWaiting / 新命令抢占），流程静默退出。
- 只读刷新（如 host 侧 profile 过期剔除）不 bump version，只发 `StateChanged(shared)`。

---

## 5. Session（状态快照，全量查询）

```rust
pub struct Session {
    state: SessionStateId,   // Waiting | HostScanning | HostStarting | HostOk | ...
    version: u64,            // index（每次 set 自增）
    shared: u64,             // 共享刷新计数
    room: Option<Room>,
    profiles: Vec<Profile>,  // HOST + LOCAL + GUEST
    port: Option<u16>,       // HostOk 的 MC 端口 / GuestOk 的本地端口
    difficulty: Option<u8>,
    error: Option<SessionError>,
    detail: String,          // 兜底 JSON 细节
}
```

`terracotta_get_state` 在 FFI 层：
```
lock(session) → 序列化 { version, state, room_code, detail(json) } → 拷入 C 结构 → 解锁
```

---

## 6. 线程与所有权模型

| 线程 | 归属 | 做什么 |
|---|---|---|
| runtime thread | 本库 | `block_on(main_loop)`，消费命令，spawn blocking 任务 |
| flow threads (spawn_blocking) | 本库 | host/guest 流程，scaffolding server 连接线程 |
| FFI 调用线程 | Conic | 只做 `try_send / lock-copy / try_recv`，**永不阻塞在库内**（poll 非阻塞） |
| easytier 子进程 | 外部 | 由 flow 持有 `Child`，drop 时 `kill()` |

- **禁止跨线程借用**：任何对外结构体都不含 Rust 内部类型；FFI 返回的字符串均为
  `Box<[u8]>`，由 `terracotta_free_string` 释放。
- **panic 边界**：所有 FFI 入口用 `catch_unwind` 包裹，出错返回 `TERRA_ERR_INTERNAL`，
  避免 panic 越过 C 边界 UB。

---

## 7. 优雅关闭（destroy）

1. `shutdown.store(true)`；
2. 向 command 队列发送 `Command::Shutdown`（唤醒 main_loop）；
3. `runtime_thread` 内部：main_loop 返回后，`spawn_blocking` 的流程线程收到 shutdown
   标志（在其同步点检查）后自行退出；EasyTier `Child` 由 `Drop` kill；
4. FFI 层 join runtime thread（带超时），随后 `Box::from_raw` 释放 context；
5. 再调用全部返回 `TERRA_ERR_INVALID_HANDLE`。

---

## 8. 与 Terracotta 的对应关系

| Terracotta | conic-terracotta |
|---|---|
| 全局 `AppState`（parking_lot Mutex） | per-context `Session`（parking_lot Mutex） |
| `AppState::acquire/set/increase` | `session.set(...)` + 发 `StateChanged` |
| `AppStateCapture.try_capture()` | `session.try_capture(token)` |
| Web UI 轮询 `GET /state` | Conic 轮询 `terracotta_poll_event` + `terracotta_get_state` |
| `set_scanning/set_guesting`（Rocket handler） | `Command::CreateRoom/JoinRoom` |
| 流程跑在 `thread::spawn` | 流程跑在 `runtime.spawn_blocking` |
| HTTP `/panic` | `Command::Shutdown` |

行为与协议（房间码、EasyTier 参数、scaffolding 线协议、LAN 组播、MOTD、异常码）
保持逐字节兼容，可与原陶瓦客户端互连。
