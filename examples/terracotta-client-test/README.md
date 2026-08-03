# terracotta-client-test — guest 示例

通过 C ABI 使用 `libconic_terracotta` 加入他人房间的客户端示例。

> 该示例**不依赖** conic-terracotta crate，也不调用其内部 Rust API。它像
> Conic Launcher 一样，用 `libloading` 动态加载动态库，只调用导出的 C 函数。

## 前置条件

- 已构建 `libconic_terracotta` 动态库，并放入 `lib/` 目录：

  ```sh
  # 在 conic-terracotta/ 下
  cargo build --release
  cp target/release/libconic_terracotta.dylib examples/terracotta-client-test/lib/
  ```

  其他平台对应产物：`conic_terracotta.dll`（Windows）、`libconic_terracotta.so`（Linux）。

## 运行

```sh
cd examples/terracotta-client-test
cargo run --release -- <ROOM_CODE>
```

示例：

```sh
cargo run --release -- U/ABCD-EFGH-JKMN-PQRS
```

## 行为

1. 校验房间号格式（`terracotta_verify_room_code`）。
2. 加载动态库，打印版本号。
3. 创建 Terracotta 上下文（`terracotta_create`）。
4. 配置 `data_dir`、`motd`（`terracotta_configure`）。
5. 加入指定房间（`terracotta_join_room`）。
6. 持续打印连接过程：状态变化（`GUEST_CONNECTING` → `GUEST_STARTING` → `GUEST_OK`）
   与事件。
7. 以 Ctrl+C 停止。

## 注意事项

- 房间的 host 必须已经在线且到达 `HOST_OK`（真实 MC 服务器被发现），guest 才能连通。
- 两个实例（host / guest）在同机运行时使用不同的 `data_dir`，互不冲突。
