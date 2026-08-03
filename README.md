# Conic Terracotta

> [!NOTE]
> 本项目所有代码均由 AI 编写，包括此 README.md 文件，但这段话是我自己写的（笑）
>
> 如果你在寻找原版陶瓦联机项目，请前往  [burningtnt/Terracotta](https://github.com/burningtnt/Terracotta)

Terracotta 联机核心的 C ABI 封装，面向 Conic Launcher 提供的**联机动态库**。

它把房间码、EasyTier 覆盖网络、scaffolding 协议等全部内部实现封装在共享库中。
上层程序不需要了解 Rust / Tokio / EasyTier 的任何细节，只需：

1. 加载动态库（`libconic_terracotta.dylib` / `.so` / `.dll`）；
2. 通过 C 接口调用。

C 语言头文件：`include/terracotta.h`（ABI 的唯一权威定义）。

---

## 目录

- [项目用途](#项目用途)
- [典型使用流程（生命周期）](#典型使用流程生命周期)
- [构建](#构建)
- [API 说明](#api-说明)
- [状态说明](#状态说明)
- [事件说明](#事件说明)
- [配置说明](#配置说明)
- [错误码](#错误码)
- [房间码格式](#房间码格式)
- [示例](#示例)

---

## 项目用途

- **联机核心**：提供 Terracotta 兼容的多人联机能力 —— 创建/加入房间、NAT 穿透、
  UDP/TCP 端口转发、玩家在线状态同步。
- **C ABI 封装**：所有功能通过一组稳定的 C 函数暴露，与语言无关，可供任何能加载
  动态库的程序（C / C++ / C# / Java / Kotlin / Rust / Python …）调用。
- **无需了解内部实现**：调用者只管「创建句柄 → 配置 → 建房间/加房间 → 轮询状态与
  事件 → 销毁」，其余全部由库内部的后台线程完成。

> 集成参考：`examples/` 下的两个示例程序展示了主程序应该如何调用本库，请保持同样的
> 调用模式。

---

## 典型使用流程（生命周期）

一个完整的生命周期如下：

1. **加载动态库**
2. **创建句柄** — `terracotta_create()`
3. **配置** — `terracotta_configure()`（`data_dir`、`public_nodes`、`motd`）
4. **创建房间或加入房间** — `terracotta_create_room()` / `terracotta_join_room()`
5. **获取状态** — 轮询 `terracotta_get_state()`
6. **获取事件** — 轮询 `terracotta_poll_event()`
7. **退出** — `terracotta_destroy()`

伪代码（语言无关）：

```c
// 1. 加载动态库（以 C 为例，实际用 dlopen / LoadLibrary / libloading 等）
void* lib = load("libconic_terracotta.dylib");

// 2. 创建句柄。失败返回 TERRA_INVALID_HANDLE。
terracotta_handle h = terracotta_create();
if (h == TERRA_INVALID_HANDLE) { /* 处理失败 */ }

// 3. 配置（必须在任何命令之前，且会话必须处于 WAITING）。
terracotta_config cfg = {
    .public_nodes      = NULL,        // 不追加额外公共节点
    .public_nodes_count= 0,
    .data_dir          = make_string("/path/to/data_dir"),  // EasyTier/machine-id 持久化
    .motd              = make_string("Ciallo～(∠・ω< )⌒★!"), // LAN 广播识别标识（可空）
};
if (terracotta_configure(h, &cfg) != TERRA_OK) { /* 处理失败 */ }

// 4a. 创建房间（room_code 传 NULL 则自动生成新房间号）
if (terracotta_create_room(h, "player_name", NULL) != TERRA_OK) { /* 处理失败 */ }
// 4b. 或者加入已有房间
// if (terracotta_join_room(h, "U/XXXX-XXXX-XXXX-XXXX", "player_name") != TERRA_OK) { ... }

// 5/6. 周期轮询（建议 ~16ms 一次）：
for (;;) {
    terracotta_event event;
    if (terracotta_poll_event(h, &event) == TERRA_OK) {
        handle_event(&event);           // 用后必须 terracotta_free_event()
    }

    terracotta_state state;
    if (terracotta_get_state(h, &state) == TERRA_OK) {
        render(state);                  // 用后必须 terracotta_free_state()
    }
}

// 7. 退出
terracotta_destroy(h);
```

---

## 构建

需要 Rust 工具链（`cargo`）。`build.rs` 会在首次构建时从 GitHub 下载对应平台的
EasyTier 二进制并嵌入库内（需要网络；之后使用本地缓存）。

```sh
cargo build --release
```

产物位置：

| 平台 | 产物 |
| --- | --- |
| macOS | `target/release/libconic_terracotta.dylib` |
| Linux | `target/release/libconic_terracotta.so` |
| Windows | `target/release/conic_terracotta.dll` |

---

## API 说明

### 通用约定

- **句柄**：`terracotta_handle` 是一个不透明指针。所有函数接收同一个句柄；销毁后
  句柄失效，再用会返回 `TERRA_ERR_INVALID_HANDLE`。
- **字符串所有权**：**所有由库返回的字符串都归库所有**，使用后必须调用对应的
  `terracotta_free_*` 释放。传入的字符串（参数）由调用方持有，无需释放。
- **非阻塞**：命令函数（`create_room` / `join_room` / `set_waiting`）只做校验并
  **同步入队后立即返回**，实际工作在库的后台线程执行；`poll_event` / `get_state`
  同样非阻塞。**不要长时间阻塞等待**，用轮询驱动 UI。
- **线程模型**：函数本身线程安全，但建议由单个逻辑线程（如启动器 UI 线程）执行
  命令、轮询与状态读取。
- **状态同步**：增量更新通过 `terracotta_poll_event()` 获取；完整快照通过
  `terracotta_get_state()` 获取。二者配合使用。

### 函数一览

#### `terracotta_create`

```c
terracotta_handle terracotta_create(void);
```

创建上下文：启动库内部的后台 runtime、命令队列与事件队列。
成功返回句柄；失败返回 `TERRA_INVALID_HANDLE`。

#### `terracotta_destroy`

```c
void terracotta_destroy(terracotta_handle handle);
```

销毁上下文：先复位当前会话（借此终止 EasyTier 子进程、停止内部假服务器），
停止内部 scaffolding 服务，再停止后台 runtime 并释放资源。可安全调用一次；
之后句柄无效。

#### `terracotta_configure`

```c
terracotta_result terracotta_configure(
    terracotta_handle        handle,
    const terracotta_config* config);
```

应用配置（`data_dir` / `public_nodes` / `motd`，见[配置说明](#配置说明)）。
必须在任何命令之前调用；若已有会话处于活动状态，返回 `TERRA_ERR_BAD_STATE`。
`config` 可传空指针或只填部分字段，未填字段使用默认值。

#### `terracotta_create_room`

```c
terracotta_result terracotta_create_room(
    terracotta_handle handle,
    const char*       player_name,  /* 可空 */
    const char*       room_code);   /* 可空 */
```

托管一个房间。`room_code` 为 `NULL` 时自动生成新房间号；否则必须是合法的
`U/XXXX-XXXX-XXXX-XXXX` 房间码，并在该网络上建房间。`player_name` 为 `NULL`
时使用默认匿名昵称。已有会话活动时返回 `TERRA_ERR_ALREADY_ACTIVE`。

#### `terracotta_join_room`

```c
terracotta_result terracotta_join_room(
    terracotta_handle handle,
    const char*       room_code,    /* 必填，格式校验 */
    const char*       player_name); /* 可空 */
```

加入已有房间。`room_code` 必填且会被校验，非法返回 `TERRA_ERR_INVALID_ROOM_CODE`。

#### `terracotta_set_waiting`

```c
terracotta_result terracotta_set_waiting(terracotta_handle handle);
```

终止当前会话并回到 `WAITING` 状态（用于退出房间/重开）。已在 `WAITING` 时为无操作。

#### `terracotta_get_state`

```c
terracotta_result terracotta_get_state(
    terracotta_handle handle,
    terracotta_state* out);
```

返回完整状态快照：`version`、`state`、`room_code`、`detail`（见[状态说明](#状态说明)）。
成功后 `*out` 中的字符串**需要**用 `terracotta_free_state()` 释放。

#### `terracotta_poll_event`

```c
terracotta_result terracotta_poll_event(
    terracotta_handle handle,
    terracotta_event* out);
```

非阻塞地取出下一条待处理事件。队列为空返回 `TERRA_ERR_NO_EVENT`（不是错误）。
成功后 `*out` 中的字符串**需要**用 `terracotta_free_event()` 释放。

#### `terracotta_verify_room_code`

```c
int terracotta_verify_room_code(const char* room_code);
```

不创建上下文即可校验房间码。合法返回 `3`，非法返回 `-1`。

#### `terracotta_version`

```c
const char* terracotta_version(void);
```

返回库版本字符串（如 `"0.1.0"`），静态存储，永不为 `NULL`，无需释放。

#### 内存释放

```c
void terracotta_free_string(terracotta_string* value);
void terracotta_free_state(terracotta_state*   value);
void terracotta_free_event(terracotta_event*   value);
```

释放由库返回并归库所有的字符串。`terracotta_free_state` / `terracotta_free_event`
会释放其内部的所有字符串字段。**每个返回的字符串都必须配对一个 free 调用**，否则
会泄漏内存。

---

## 状态说明

`terracotta_state.state` 取值如下（也对应 `terracotta_event` 中
`STATE_CHANGED` 的 `state` 字段）：

| id | 枚举名 | 含义 | `detail` 内容 |
| --- | --- | --- | --- |
| 0 | `WAITING` | 空闲，可以发起创建/加入房间 | `{}` |
| 1 | `HOST_SCANNING` | 主机正在通过组播扫描局域网内的真实 MC 服务器 | `{}` |
| 2 | `HOST_STARTING` | 已发现服务器，正在启动 EasyTier 覆盖网络 | `{}` |
| 3 | `HOST_OK` | 主机就绪，房间可被加入 | `{"port":N,"profiles":[...]}` |
| 4 | `GUEST_CONNECTING` | 客户端已提交加入，正在连接 EasyTier 网络 | `{}` |
| 5 | `GUEST_STARTING` | 客户端正在与主机 scaffolding 握手、建立端口转发 | `{}` |
| 6 | `GUEST_OK` | 客户端就绪，可以连接 `url` 进入游戏 | `{"url":"127.0.0.1:port","profiles":[...]}` |
| 7 | `EXCEPTION` | 会话异常终止 | `{"error":{"code":N,"message":"..."}}` |

> `profiles` 数组元素：`{"machine_id":"...","name":"...","vendor":"...","kind":"HOST"|"LOCAL"|"GUEST"}`。

---

## 事件说明

事件通过 `terracotta_poll_event()` 获取。`type` 取值与 `payload`（JSON 字符串）如下：

| type | 枚举名 | payload 示例 | 用途 |
| --- | --- | --- | --- |
| 1 | `STATE_CHANGED` | `{"state":1,"version":3}` | 会话状态发生变化（配合状态表解读） |
| 2 | `PLAYER_JOINED` | `{"profile":{"machine_id":"...","name":"...","vendor":"...","kind":"GUEST"}}` | 有玩家加入房间 |
| 3 | `PLAYER_LEFT` | `{"machine_id":"..."}` | 有玩家离开房间 |
| 4 | `CONNECTION_DIFFICULTY` | `{"difficulty":0}` | 客户端与主机的连接难度（0=未知,1=极易,2=简单,3=中等,4=困难） |
| 5 | `HOST_READY` | `{"room":"U/XXXX-...","port":25565}` | 主机就绪，发布房间号与 MC 端口 |
| 6 | `GUEST_READY` | `{"url":"127.0.0.1:25565"}` | 客户端就绪，给出可连接的本地地址 |
| 7 | `ERROR` | `{"code":1,"message":"..."}` | 命令/会话级错误 |

> 每个事件都有单调递增的 `sequence`，可用来保证 UI 更新的先后顺序。

---

## 配置说明

`terracotta_config` 包含三个字段：

### `data_dir`

- EasyTier 二进制的解压位置、`machine-id` 文件等持久化数据的存放目录。
- **可空**：为空时使用系统临时目录下的默认路径。
- 建议为每个实例指定独立的 `data_dir`，这样同一台机器上可同时运行多个
  Terracotta 实例（例如 host 与 guest），互不冲突。

### `public_nodes`

- 指定额外的 EasyTier 公共节点（`terracotta_string` 数组，`public_nodes_count`
  为个数）。
- **可空**：传 `NULL` + `count = 0` 即可；库内部始终会追加一组内置默认公共节点。
- 用于自建公共节点或内网环境中引导覆盖网络。

### `motd`

- 用于 Minecraft LAN 广播识别的一个标识字符串。
- 主机侧：扫描局域网服务器时会**忽略 MOTD 恰好等于该值**的服务器（即自己或其它
  Terracotta 实例的虚拟服务器），只托管真实的 MC 服务器。
- 客户端侧：会用该值广播一个虚拟 MC 服务器，使局域网内的 MC 客户端能看到房间。
- **可空**：为空时使用内置默认 MOTD。

---

## 错误码

| 值 | 名称 | 含义 |
| --- | --- | --- |
| 0 | `TERRA_OK` | 成功 |
| -1 | `TERRA_ERR_INVALID_HANDLE` | 句柄为 `NULL` 或已销毁 |
| -2 | `TERRA_ERR_INVALID_ARGUMENT` | 参数非法（NULL/空串/长度错误等） |
| -3 | `TERRA_ERR_BAD_STATE` | 当前状态下不允许该命令（如会话活动时 `configure`） |
| -4 | `TERRA_ERR_INVALID_ROOM_CODE` | 房间码格式非法 |
| -5 | `TERRA_ERR_ALREADY_ACTIVE` | 已有会话在运行，无法再创建/加入 |
| -6 | `TERRA_ERR_INTERNAL` | 内部错误（库内 panic 已被兜底） |
| -7 | `TERRA_ERR_OUT_OF_MEMORY` | 内存不足 |
| -8 | `TERRA_ERR_NO_EVENT` | `poll_event` 队列为空（非错误） |
| -9 | `TERRA_ERR_SHUTTING_DOWN` | 销毁正在进行中 |

---

## 房间码格式

- 形如 `U/XXXX-XXXX-XXXX-XXXX`，共 21 字符。
- 字符集为 `0-9 A-Z`，且**不含易混淆的 `I` 与 `O`**。
- 16 位 base-34 数值是 `7` 的倍数（用于自校验）。
- 房间码同时编码了 EasyTier 的 `network_name` 与 `network_secret`，因此客户端
  只需拿到房间码即可加入对应网络。

---

## 示例

`examples/` 下有两个可直接运行的程序，展示了主程序应如何使用本库（动态加载 +
C ABI，不依赖 crate 内部 API）：

> [!IMPORTANT]
> `examples/`目录下仅包含 MacOS aarch64 平台的编译好的库，如果需要在其他环境运行示例，请先构建项目，并把构建产物放在 `examples/<示例名>/lib` 目录下，MacOS 请确保文件名为 `libconic_terracotta.dylib`，Windows 需确保文件名为 `conic_terracotta.dll`，Linux 需确保文件名为 `libconic_terracotta.so`

| 示例 | 作用 | 运行 |
| --- | --- | --- |
| `examples/terracotta-test` | 主机：创建房间、输出房间号与状态、等待玩家 | `cd examples/terracotta-test && cargo run --release` |
| `examples/terracotta-client-test` | 客户端：加入指定房间、输出连接过程 | `cd examples/terracotta-client-test && cargo run --release -- U/XXXX-XXXX-XXXX-XXXX` |

详见各示例目录下的 `README.md`。

## 致谢

Conic Terracotta 是原 [陶瓦（Terracotta）](https://github.com/burningtnt/Terracotta)（作者 burningtnt，[AGPL-3.0-or-later 许可](https://www.gnu.org/licenses/agpl-3.0.html)）的**派生与移植作品**：房间码协议、scaffolding 网络协议、局域网联机状态机、EasyTier 参数/公共节点等大量代码直接移植自原项目，而非仅受其启发。

原项目版权归 burningtnt 及其贡献者所有，本仓库以 `THIRD_PARTY_LICENSE` 附上其完整许可文本，并在所有直接移植的源文件头部保留了来源与版权归属说明。依照 AGPL 第 5 节要求，本作品在显著位置标注了上述修改与来源，并继续以 AGPL 许可分发。

感谢陶瓦项目的作者和贡献者为 Minecraft 联机社区所做的工作，以及他们以自由软件许可向社区开放这一项目。

本项目同时使用 [EasyTier](https://github.com/EasyTier/EasyTier/) 作为底层虚拟网络方案。感谢 EasyTier 的作者和贡献者提供了稳定的点对点网络能力，使跨网络 Minecraft 局域网联机成为可能。

Conic Terracotta 是一个独立项目，与原陶瓦项目及其作者不存在隶属、维护或官方合作关系。
