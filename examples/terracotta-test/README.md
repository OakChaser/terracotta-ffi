# terracotta-test — host 示例

通过 C ABI 使用 `libconic_terracotta` 创建并托管一个联机房间的主机示例。

> 该示例**不依赖** conic-terracotta crate，也不调用其内部 Rust API。它像
> Conic Launcher 一样，用 `libloading` 动态加载动态库，只调用导出的 C 函数。

## 前置条件

- 已构建 `libconic_terracotta` 动态库，并放入 `lib/` 目录：

  ```sh
  # 在 conic-terracotta/ 下
  cargo build --release
  cp target/release/libconic_terracotta.dylib examples/terracotta-test/lib/
  ```

  其他平台对应产物：`conic_terracotta.dll`（Windows）、`libconic_terracotta.so`（Linux）。

## 运行

```sh
cd examples/terracotta-test
cargo run --release
```

## 行为

1. 加载动态库，打印版本号。
2. 创建 Terracotta 上下文（`terracotta_create`）。
3. 配置 `data_dir`、`motd`（`terracotta_configure`）。
4. 创建房间（`terracotta_create_room`）。
5. 打印房间号（形如 `U/XXXX-XXXX-XXXX-XXXX`）。
6. 持续打印状态变化与事件，等待其他玩家加入。
7. 以 Ctrl+C 停止。

## 让其他玩家加入

把输出的房间号复制给另一台机器上的 guest 示例：

```sh
terracotta-client-test U/XXXX-XXXX-XXXX-XXXX
```

> 提示：主机必须先达到 `HOST_OK`（已通过组播扫描发现真实的 Minecraft 服务器），
> guest 才能连接上。
