# Systemd Deployment

## Prerequisites
- linux box with systemd
- node.js and npm

## Build

Follow the instructions in the top-level README to build. Or download a release
artifact.

## Setup

1. **Create hooya user:**
```bash
sudo useradd -r -s /bin/false hooya
```

2. **Install hooyad and hooya-web-proxy:**
```bash
sudo mkdir -p /opt/hooya/bin /opt/hooya/filestore
sudo cp target/release/hooyad /opt/hooya/bin/
sudo cp target/release/hooya-web-proxy /opt/hooya/bin/
sudo chown -R hooya:hooya /opt/hooya
```

3. **Install hooya-web-ui:**
```bash
sudo mkdir -p /opt/hooya-web-ui
sudo cp -r .next/standalone/* /opt/hooya-web-ui/
sudo cp -r .next/static /opt/hooya-web-ui/.next/
sudo cp -r public /opt/hooya-web-ui/
sudo chown -R hooya:hooya /opt/hooya-web-ui
```

4. **Install services:**
```bash
sudo cp *.service /etc/systemd/system/
sudo systemctl daemon-reload
```

5. **Start services:**
```bash
sudo systemctl enable --now hooyad
sudo systemctl enable --now hooya-web-proxy
sudo systemctl enable --now hooya-web-ui
```

## Access
- Web UI: http://localhost:3000
- Web Proxy API: http://localhost:8532
- Hooya Daemon: gRPC on localhost:8531

## Management
```bash
# Check status
sudo systemctl status hooyad hooya-web-proxy hooya-web-ui

# View logs
sudo journalctl -u hooyad -f
```

## Get Login Password
The web proxy generates a random password on first startup. To see it:
```bash
sudo journalctl -u hooya-web-proxy | grep "generated operator password"
```

To set a custom password:
```bash
sudo -u hooya /opt/hooya/bin/hooya-web-proxy set-password yourpassword
```