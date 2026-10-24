# Nix / NixOS

仓库提供一个 Flake，包含服务端 `hireferry` 和终端客户端 `hfy`。当前支持
`x86_64-linux` 与 `aarch64-linux`；Flake 会使用仓库自己的 `Cargo.lock`，不会把
本地配置、数据库或密钥带进构建。

## 直接安装

```sh
nix profile install github:TokenNotIncluded/hireferry#cli
hfy --version
```

安装自托管服务端：

```sh
nix profile install github:TokenNotIncluded/hireferry#server
hireferry --version
```

`server` 只提供二进制，不替你创建 systemd 用户、配置文件或数据库。生产部署仍按
[自托管指南](self-hosting.md)准备 `config.toml`、权限、备份和反向代理。

## 从 checkout 构建

```sh
git clone https://github.com/TokenNotIncluded/hireferry.git
cd hireferry
nix build .#cli
./result/bin/hfy --version
nix build .#server
./result/bin/hireferry --version
```

开发 shell：

```sh
nix develop
```

验证 Flake 的元数据和构建入口：

```sh
nix flake check
```

这是源码构建渠道，不等于 GitHub Release 的签名、平台公证或实机验收。正式下载包仍以
[发布矩阵](downloads.md)和对应 Release 的 `SHA256SUMS` 为准。
