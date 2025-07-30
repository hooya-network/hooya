hooya
=====

P2P [booru](https://en.wiktionary.org/wiki/booru). It associates images and
videos with tag metadata.

<figure>
<a href="https://public-demo.hooya.wesl.ee/cid/bafkreihsz45vfhczlhihjbg6imrbtmrbaoeepcjvhfuhkjktaxzrw6vroy">
<img class="hooya-img-medium" src="https://web.hooya.wesl.ee/cid-thumbnail/bafkreicflu5adp3sitaqqclujiy4cuyqkhdom2irhpu366gym4q3niiega/medium">
</a>
<figcaption><p>This image, itself, is hosted with HooYa.</p></figcaption>
</figure>

Longer discussion on what boorus are and on the HooYa vision is at
[wesl.ee/HooYa](https://wesl.ee/HooYa/).

## Components

- **[hooyad](crates/hooyad/)** - Core daemon for file storage. Handles P2P connectivity
- **[hooya-web-proxy](crates/hooya-web-proxy/)** - HTTP proxy server that provides REST API access to
  hooyad for clients like hooya-web-ui and hooya-client
- **[hooya-web-ui](https://github.com/hooya-network/hooya-web-ui)** - Next.js
  web interface for browsing and managing files
- **[hooya-client](crates/hooya-client/)** - CLI client for interacting with
  hooyad. Probably deprecated soon because web-ui is just better.
- **[hooya-gtk](hooya-gtk/)** - GTK desktop application. Very dead. Deader than
  the CLI client.
- **[proto](https://github.com/hooya-network/hooya-protobuf)** - Protocol buffer
  definitions for gRPC communication

Installation
------------

Use `nix develop` to boot into a dev shell:

```bash
git clone --recurse-submodules git@github.com:hooya-network/hooya.git
cd hooya
nix develop
cargo build --release
```

For the web UI ([separate repository](https://github.com/hooya-network/hooya-web-ui)):

```bash
git clone git@github.com:hooya-network/hooya-web-ui.git
cd hooya-web-ui
nix develop
npm i
npm run dev
# or npm run build
```

See the [hooya-web-ui README](https://github.com/hooya-network/hooya-web-ui#readme) for detailed setup instructions.

Running
-------

### Core Services

Start the core daemon:
```bash
./target/release/hooyad
```

**Note**: Secure access to the gRPC control port (TCP 8531) otherwise anyone can
connect and upload anything to your instance.

Start the web proxy

```bash
./target/release/hooya-web-proxy
```

### Web Interface

Start the web UI development server ([hooya-web-ui](https://github.com/hooya-network/hooya-web-ui)):

```bash
cd ../hooya-web-ui
npm run dev
```

The web interface will be available at http://localhost:3000 and connects to the web proxy on port 8532.


Deploy
------

Production deployment examples are available in the [`/deploy`](deploy/) directory:

- **[Docker Compose](deploy/docker-compose/)**: Containerized deployment with pre-built images
- **[Kubernetes](deploy/k8s/)**: Kubernetes manifests for cluster deployment  
- **[systemd](deploy/systemd/)**: systemd service files for Linux systems

Each deployment method includes both the core hooya services and the web UI.

License
-------

MIT License (available in the source tree as /LICENSE)
