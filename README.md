hooya
=====

Local booru with P2P aspirations. Presently handicapped by the time I can afford
to sink into this project.

Installation
------------

Install dependencies yourself or use `nix-shell` to manage them.

```
git clone git@github.com:hooya-network/hooya.git
cd hooya
nix-shell
cargo build --release
```

Running
-------

Run `hooyad` locally or in a cloud somewhere and connect to it via the provided client.
Client-server communication afforded by protobuf and gRPC. Secure access to its
control RPC port TCP 8531 unless you want anyone to be able to connect and
upload anything to the instance, not recommended.

```
# Run the daemon
./target/release/hooyad
# Communicate with the daemon from a remote client (endpoint optional)
./target/release/hooya --endpoint <endpoint> add <PATH-TO-FILE>
```

License
-------

MIT License (available in the source tree as /LICENSE)
