# Docker Compose Deployment

## Prerequisites
- Docker and Docker Compose installed
- Pre-built images: `hooya:latest`, `hooya-web-proxy:latest`, `hooya-web-ui:latest`

## Deploy
```bash
docker-compose up -d
```

## Access
- Web UI: http://localhost:3000
- Web Proxy API: http://localhost:8532
- Hooya Daemon: gRPC on localhost:8531

## Get Login Password
The web proxy generates a random password on first startup. To see it:
```bash
docker-compose logs hooya-web-proxy | grep "generated operator password"
```

To set a custom password:
```bash
docker-compose exec hooya-web-proxy /hooya-web-proxy set-password yourpassword
```

## Stop
```bash
docker-compose down
```

## Data Persistence
Data is stored in the `hooya_data` Docker volume and will persist across restarts.