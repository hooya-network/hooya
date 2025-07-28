# Kubernetes Deployment

## Prerequisites
- Kubernetes cluster (minikube, k3s, or cloud provider)
- kubectl configured
- Docker for building images
- Container registry access

## Deploy
```bash
kubectl apply -f manifests.yml
```

## Access
```bash
# Get web UI service external IP
kubectl get service hooya-web-ui-service

# Or port-forward for local access
kubectl port-forward service/hooya-web-ui-service 3000:3000
```

## Management
```bash
# Check pods
kubectl get pods

# View logs
kubectl logs -l app=hooyad
kubectl logs -l app=hooya-web-proxy
kubectl logs -l app=hooya-web-ui
```

## Clean Up
```bash
kubectl delete -f manifests.yml
```