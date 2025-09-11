use anyhow::{Context, Result};
use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec};
use k8s_openapi::api::core::v1::{
    ConfigMap, Container, ContainerPort, PersistentVolumeClaim,
    PersistentVolumeClaimSpec, PodSpec, PodTemplateSpec, Service, ServicePort,
    ServiceSpec, Volume, VolumeMount, VolumeResourceRequirements,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector;
use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
use kube::api::{DeleteParams, PostParams};
use kube::{Api, Client};
use std::collections::BTreeMap;
use std::time::Duration;
use tracing::{debug, info, warn};

// Docker images for integration testing
const HOOYAD_IMAGE: &str = "hooyad:itests";
const HOOYA_WEB_PROXY_IMAGE: &str = "hooya-web-proxy:itests";

#[derive(Debug, Clone)]
pub struct NodeConfig {
    pub name: String,
    pub operator: String,
    pub instance_name: String,
    pub db_uri: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClusterTopology {
    pub nodes: Vec<NodeConfig>,
    pub proxies: u32,
    pub namespace: String,
}

pub struct DeployedCluster {
    pub topology: ClusterTopology,
    pub namespace: String,
    pub client: Client,
    pub node_endpoints: Vec<String>,
    pub proxy_endpoints: Vec<String>,
    pub proxy_passwords: Vec<Option<String>>,
    pub node_enrs: Vec<Option<String>>,
}

impl DeployedCluster {
    /// Get the REST endpoint for a specific proxy
    pub fn proxy_url(&self, proxy_index: usize) -> Option<String> {
        self.proxy_endpoints.get(proxy_index).cloned()
    }

    /// Get all proxy URLs
    pub fn proxy_urls(&self) -> &[String] {
        &self.proxy_endpoints
    }

    /// Get the password for a specific proxy
    pub fn proxy_password(&self, proxy_index: usize) -> Option<&String> {
        self.proxy_passwords
            .get(proxy_index)
            .and_then(|p| p.as_ref())
    }

    /// Get the ENR for a specific node
    pub fn node_enr(&self, node_index: usize) -> Option<&String> {
        self.node_enrs.get(node_index).and_then(|enr| enr.as_ref())
    }

    /// Extract the ENR from a node's logs
    pub async fn extract_node_enr(
        &self,
        node_index: usize,
    ) -> Result<Option<String>> {
        use k8s_openapi::api::core::v1::Pod;
        use kube::api::LogParams;

        let pods: Api<Pod> =
            Api::namespaced(self.client.clone(), &self.namespace);

        // Find the node pod
        let label_selector = format!("app=hooyad-node-{node_index}");
        let list_params =
            kube::api::ListParams::default().labels(&label_selector);

        let pod_list = pods
            .list(&list_params)
            .await
            .context("Failed to list pods")?;

        if let Some(pod) = pod_list.items.first() {
            if let Some(pod_name) = &pod.metadata.name {
                info!("Extracting ENR from node pod: {}", pod_name);

                // Get logs from the container
                let log_params = LogParams {
                    container: Some("hooyad".to_string()),
                    ..Default::default()
                };

                let logs = pods
                    .logs(pod_name, &log_params)
                    .await
                    .context("Failed to get pod logs")?;

                // Search for ENR patterns - look for both initial and updated ENRs
                let mut latest_enr = None;
                for line in logs.lines() {
                    // Look for discv5 starting with ENR (initial ENR)
                    if line.contains("discv5 starting")
                        && line.contains("local_enr=")
                    {
                        if let Some(enr_part) = line.split("local_enr=").nth(1)
                        {
                            let enr = enr_part
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            if enr.starts_with("enr:") {
                                info!(
                                    "Found initial ENR for node {}: {}",
                                    node_index, enr
                                );
                                latest_enr = Some(enr);
                            }
                        }
                    }
                    // Look for updated ENR from mDNS + identify (this overrides initial ENR)
                    if line.contains("updated enr after mdns identify")
                        && line.contains("enr=")
                    {
                        if let Some(enr_part) = line.split("enr=").nth(1) {
                            let enr = enr_part
                                .split_whitespace()
                                .next()
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            if enr.starts_with("enr:") {
                                info!("Found updated ENR for node {} (mDNS + identify): {}", node_index, enr);
                                latest_enr = Some(enr);
                            }
                        }
                    }
                }

                if let Some(enr) = latest_enr {
                    return Ok(Some(enr));
                }

                warn!("No ENR found in node {} logs", node_index);
            }
        } else {
            warn!("No node pod found for index {}", node_index);
        }

        Ok(None)
    }

    /// Extract the generated password from a proxy's logs
    pub async fn extract_proxy_password(
        &self,
        proxy_index: u32,
    ) -> Result<Option<String>> {
        use k8s_openapi::api::core::v1::Pod;
        use kube::api::LogParams;

        let pods: Api<Pod> =
            Api::namespaced(self.client.clone(), &self.namespace);

        // Find the proxy pod
        let label_selector = format!("app=hooya-web-proxy-{proxy_index}");
        let list_params =
            kube::api::ListParams::default().labels(&label_selector);

        let pod_list = pods
            .list(&list_params)
            .await
            .context("Failed to list pods")?;

        if let Some(pod) = pod_list.items.first() {
            if let Some(pod_name) = &pod.metadata.name {
                info!("Extracting password from proxy pod: {}", pod_name);

                // Get logs from the container
                let log_params = LogParams {
                    container: Some("hooya-web-proxy".to_string()),
                    ..Default::default()
                };

                let logs = pods
                    .logs(pod_name, &log_params)
                    .await
                    .context("Failed to get pod logs")?;

                // Search for the password pattern
                for line in logs.lines() {
                    if line.contains("generated operator password:") {
                        if let Some(password) =
                            line.split("generated operator password:").nth(1)
                        {
                            let password = password.trim().to_string();
                            info!(
                                "Found generated password for proxy {}: {}",
                                proxy_index, password
                            );
                            return Ok(Some(password));
                        }
                    }
                }

                warn!(
                    "No generated password found in proxy {} logs",
                    proxy_index
                );
            }
        } else {
            warn!("No proxy pod found for index {}", proxy_index);
        }

        Ok(None)
    }

    /// Cleanup the entire cluster
    pub async fn cleanup(&self) -> Result<()> {
        info!("Cleaning up test cluster in namespace: {}", self.namespace);

        let delete_params = DeleteParams::default();

        // Delete deployments
        let deployments: Api<Deployment> =
            Api::namespaced(self.client.clone(), &self.namespace);
        for node in &self.topology.nodes {
            let deployment_name = format!("hooyad-{}", node.name);
            if let Err(e) =
                deployments.delete(&deployment_name, &delete_params).await
            {
                warn!("Failed to delete deployment {}: {}", deployment_name, e);
            }
        }

        for i in 0..self.topology.proxies {
            let deployment_name = format!("hooya-web-proxy-{i}");
            if let Err(e) =
                deployments.delete(&deployment_name, &delete_params).await
            {
                warn!("Failed to delete deployment {}: {}", deployment_name, e);
            }
        }

        // Delete services
        let services: Api<Service> =
            Api::namespaced(self.client.clone(), &self.namespace);
        for node in &self.topology.nodes {
            let service_name = format!("hooyad-{}-service", node.name);
            if let Err(e) = services.delete(&service_name, &delete_params).await
            {
                warn!("Failed to delete service {}: {}", service_name, e);
            }
        }

        for i in 0..self.topology.proxies {
            let service_name = format!("hooya-web-proxy-{i}-service");
            if let Err(e) = services.delete(&service_name, &delete_params).await
            {
                warn!("Failed to delete service {}: {}", service_name, e);
            }
        }

        // Delete ConfigMaps
        let configmaps: Api<ConfigMap> =
            Api::namespaced(self.client.clone(), &self.namespace);
        for node in &self.topology.nodes {
            let configmap_name = format!("hooyad-{}-config", node.name);
            if let Err(e) =
                configmaps.delete(&configmap_name, &delete_params).await
            {
                warn!("Failed to delete configmap {}: {}", configmap_name, e);
            }
        }

        // Delete PVCs
        let pvcs: Api<PersistentVolumeClaim> =
            Api::namespaced(self.client.clone(), &self.namespace);
        for node in &self.topology.nodes {
            let pvc_name = format!("hooyad-{}-pvc", node.name);
            if let Err(e) = pvcs.delete(&pvc_name, &delete_params).await {
                warn!("Failed to delete pvc {}: {}", pvc_name, e);
            }
        }

        info!("Cluster cleanup completed");
        Ok(())
    }
}

pub async fn create_test_cluster(
    topology: ClusterTopology,
) -> Result<DeployedCluster> {
    let client = Client::try_default()
        .await
        .context("Failed to create Kubernetes client")?;

    info!(
        "Creating test cluster with {} nodes and {} proxies in namespace: {}",
        topology.nodes.len(),
        topology.proxies,
        topology.namespace
    );

    // Create namespace if it doesn't exist
    ensure_namespace(&client, &topology.namespace).await?;

    let mut node_endpoints = Vec::new();
    let mut proxy_endpoints = Vec::new();

    // Deploy each hooyad node
    for (i, node) in topology.nodes.iter().enumerate() {
        deploy_hooyad_node(&client, &topology.namespace, node, i).await?;
        node_endpoints.push(format!(
            "hooyad-{}-service.{}.svc.cluster.local:8531",
            node.name, topology.namespace
        ));
    }

    // Wait for nodes to be ready
    wait_for_deployments_ready(&client, &topology.namespace, &topology.nodes)
        .await?;

    // Deploy proxies, mapping each proxy to a specific node (round-robin)
    for i in 0..topology.proxies {
        let target_node_index = (i as usize) % topology.nodes.len();
        deploy_hooya_proxy(&client, &topology.namespace, i, target_node_index)
            .await?;
        proxy_endpoints.push(format!(
            "hooya-web-proxy-{}-service.{}.svc.cluster.local:8532",
            i, topology.namespace
        ));
    }

    // Wait for proxies to be ready
    wait_for_proxy_deployments_ready(
        &client,
        &topology.namespace,
        topology.proxies,
    )
    .await?;

    // Create initial cluster without passwords
    let cluster = DeployedCluster {
        topology: topology.clone(),
        namespace: topology.namespace.clone(),
        client,
        node_endpoints,
        proxy_endpoints,
        proxy_passwords: vec![None; topology.proxies as usize],
        node_enrs: vec![None; topology.nodes.len()],
    };

    // Extract passwords from proxy logs
    let mut proxy_passwords = Vec::new();
    for i in 0..topology.proxies {
        info!("Extracting password for proxy {}", i);
        let password = cluster.extract_proxy_password(i).await?;
        proxy_passwords.push(password);
    }

    // Extract ENRs from node logs
    let mut node_enrs = Vec::new();
    for i in 0..topology.nodes.len() {
        info!("Extracting ENR for node {}", i);
        let enr = cluster.extract_node_enr(i).await?;
        node_enrs.push(enr);
    }

    Ok(DeployedCluster {
        topology: topology.clone(),
        namespace: topology.namespace.clone(),
        client: cluster.client,
        node_endpoints: cluster.node_endpoints,
        proxy_endpoints: cluster.proxy_endpoints,
        proxy_passwords,
        node_enrs,
    })
}

async fn ensure_namespace(client: &Client, namespace: &str) -> Result<()> {
    use k8s_openapi::api::core::v1::Namespace;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    let namespaces: Api<Namespace> = Api::all(client.clone());

    if namespaces.get(namespace).await.is_ok() {
        debug!("Namespace {} already exists", namespace);
        return Ok(());
    }

    let ns = Namespace {
        metadata: ObjectMeta {
            name: Some(namespace.to_string()),
            ..Default::default()
        },
        ..Default::default()
    };

    namespaces
        .create(&PostParams::default(), &ns)
        .await
        .context("Failed to create namespace")?;

    info!("Created namespace: {}", namespace);
    Ok(())
}

async fn deploy_hooyad_node(
    client: &Client,
    namespace: &str,
    node: &NodeConfig,
    index: usize,
) -> Result<()> {
    // Create ConfigMap for hooya.toml
    create_hooyad_config(client, namespace, node).await?;

    // Create PVC for data
    create_hooyad_pvc(client, namespace, node).await?;

    // Create Deployment
    create_hooyad_deployment(client, namespace, node, index).await?;

    // Create Service
    create_hooyad_service(client, namespace, node).await?;

    Ok(())
}

async fn create_hooyad_config(
    client: &Client,
    namespace: &str,
    node: &NodeConfig,
) -> Result<()> {
    let filestore_section = if let Some(db_uri) = &node.db_uri {
        format!(
            r#"[filestore]
db_uri = "{db_uri}"

"#
        )
    } else {
        String::new()
    };

    let config_content = format!(
        r#"[instance]
name = "{}"
operator = "{}"

{}[networking]
max_peers = 50
max_message_size_bytes = 1048576
listen_addresses = [
  "/ip4/0.0.0.0/tcp/8530",
  "/ip6/::/tcp/8530"
]
advertise_addresses = []
discv5_listen_addresses = [
  "/ip4/0.0.0.0/udp/8530", 
  "/ip6/::/udp/8530"
]

[networking.discovery]
discovery_interval_secs = 30

[networking.discovery.mdns]
enabled = true
service_name = "hooya-mesh"
discovery_interval_secs = 30

[networking.discovery.dns]
enabled = false
bootstrap_domain = "bootstrap.hooya.org"
"#,
        node.instance_name, node.operator, filestore_section
    );

    let mut data = BTreeMap::new();
    data.insert("hooya.toml".to_string(), config_content);

    let configmap = ConfigMap {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(format!("hooyad-{}-config", node.name)),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    };

    let configmaps: Api<ConfigMap> = Api::namespaced(client.clone(), namespace);
    configmaps
        .create(&PostParams::default(), &configmap)
        .await
        .context("Failed to create hooyad config")?;

    debug!("Created config for node: {}", node.name);
    Ok(())
}

async fn create_hooyad_pvc(
    client: &Client,
    namespace: &str,
    node: &NodeConfig,
) -> Result<()> {
    let mut requests = BTreeMap::new();
    requests.insert("storage".to_string(), Quantity("1Gi".to_string()));

    let pvc = PersistentVolumeClaim {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(format!("hooyad-{}-pvc", node.name)),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOnce".to_string()]),
            resources: Some(VolumeResourceRequirements {
                requests: Some(requests),
                ..Default::default()
            }),
            ..Default::default()
        }),
        ..Default::default()
    };

    let pvcs: Api<PersistentVolumeClaim> =
        Api::namespaced(client.clone(), namespace);
    pvcs.create(&PostParams::default(), &pvc)
        .await
        .context("Failed to create PVC")?;

    debug!("Created PVC for node: {}", node.name);
    Ok(())
}

async fn create_hooyad_deployment(
    client: &Client,
    namespace: &str,
    node: &NodeConfig,
    _index: usize,
) -> Result<()> {
    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), format!("hooyad-{}", node.name));

    let deployment = Deployment {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(format!("hooyad-{}", node.name)),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(1),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "hooyad".to_string(),
                        image: Some(HOOYAD_IMAGE.to_string()),
                        ports: Some(vec![
                            ContainerPort {
                                container_port: 8531,
                                name: Some("grpc".to_string()),
                                ..Default::default()
                            },
                            ContainerPort {
                                container_port: 8530,
                                name: Some("p2p-tcp".to_string()),
                                protocol: Some("TCP".to_string()),
                                ..Default::default()
                            },
                            ContainerPort {
                                container_port: 8530,
                                name: Some("p2p-udp".to_string()),
                                protocol: Some("UDP".to_string()),
                                ..Default::default()
                            },
                        ]),
                        env: Some({
                            let env_vars = vec![
                                k8s_openapi::api::core::v1::EnvVar {
                                    name: "HOOYAD_ENDPOINT".to_string(),
                                    value: Some("0.0.0.0:8531".to_string()),
                                    ..Default::default()
                                },
                                k8s_openapi::api::core::v1::EnvVar {
                                    name: "HOOYAD_FILESTORE".to_string(),
                                    value: Some("/data".to_string()),
                                    ..Default::default()
                                },
                                k8s_openapi::api::core::v1::EnvVar {
                                    name: "HOOYAD_LOG_LEVEL".to_string(),
                                    value: Some("debug".to_string()),
                                    ..Default::default()
                                },
                            ];


                            env_vars
                        }),
                        volume_mounts: Some(vec![
                            VolumeMount {
                                name: "data".to_string(),
                                mount_path: "/data".to_string(),
                                ..Default::default()
                            },
                            VolumeMount {
                                name: "config".to_string(),
                                mount_path: "/data/hooya.toml".to_string(),
                                sub_path: Some("hooya.toml".to_string()),
                                ..Default::default()
                            },
                        ]),
                        ..Default::default()
                    }],
                    volumes: Some(vec![
                        Volume {
                            name: "data".to_string(),
                            persistent_volume_claim: Some(
                                k8s_openapi::api::core::v1::PersistentVolumeClaimVolumeSource {
                                    claim_name: format!("hooyad-{}-pvc", node.name),
                                    ..Default::default()
                                },
                            ),
                            ..Default::default()
                        },
                        Volume {
                            name: "config".to_string(),
                            config_map: Some(
                                k8s_openapi::api::core::v1::ConfigMapVolumeSource {
                                    name: format!("hooyad-{}-config", node.name),
                                    ..Default::default()
                                },
                            ),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    };

    let deployments: Api<Deployment> =
        Api::namespaced(client.clone(), namespace);
    deployments
        .create(&PostParams::default(), &deployment)
        .await
        .context("Failed to create hooyad deployment")?;

    debug!("Created deployment for node: {}", node.name);
    Ok(())
}

async fn create_hooyad_service(
    client: &Client,
    namespace: &str,
    node: &NodeConfig,
) -> Result<()> {
    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), format!("hooyad-{}", node.name));

    let service = Service {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(format!("hooyad-{}-service", node.name)),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(labels),
            ports: Some(vec![
                ServicePort {
                    name: Some("grpc".to_string()),
                    port: 8531,
                    target_port: Some(IntOrString::Int(8531)),
                    ..Default::default()
                },
                ServicePort {
                    name: Some("p2p-tcp".to_string()),
                    port: 8530,
                    target_port: Some(IntOrString::Int(8530)),
                    protocol: Some("TCP".to_string()),
                    ..Default::default()
                },
                ServicePort {
                    name: Some("p2p-udp".to_string()),
                    port: 8530,
                    target_port: Some(IntOrString::Int(8530)),
                    protocol: Some("UDP".to_string()),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let services: Api<Service> = Api::namespaced(client.clone(), namespace);
    services
        .create(&PostParams::default(), &service)
        .await
        .context("Failed to create hooyad service")?;

    debug!("Created service for node: {}", node.name);
    Ok(())
}

async fn deploy_hooya_proxy(
    client: &Client,
    namespace: &str,
    proxy_index: u32,
    target_node_index: usize,
) -> Result<()> {
    create_proxy_deployment(client, namespace, proxy_index, target_node_index)
        .await?;
    create_proxy_service(client, namespace, proxy_index).await?;
    Ok(())
}

async fn create_proxy_deployment(
    client: &Client,
    namespace: &str,
    proxy_index: u32,
    target_node_index: usize,
) -> Result<()> {
    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), format!("hooya-web-proxy-{proxy_index}"));

    let deployment = Deployment {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(format!("hooya-web-proxy-{proxy_index}")),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(DeploymentSpec {
            replicas: Some(1),
            selector: LabelSelector {
                match_labels: Some(labels.clone()),
                ..Default::default()
            },
            template: PodTemplateSpec {
                metadata: Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
                    labels: Some(labels),
                    ..Default::default()
                }),
                spec: Some(PodSpec {
                    containers: vec![Container {
                        name: "hooya-web-proxy".to_string(),
                        image: Some(HOOYA_WEB_PROXY_IMAGE.to_string()),
                        ports: Some(vec![ContainerPort {
                            container_port: 8532,
                            name: Some("http".to_string()),
                            ..Default::default()
                        }]),
                        env: Some(vec![
                            k8s_openapi::api::core::v1::EnvVar {
                                name: "HOOYA_WEB_PROXY_ENDPOINT".to_string(),
                                value: Some("0.0.0.0:8532".to_string()),
                                ..Default::default()
                            },
                            k8s_openapi::api::core::v1::EnvVar {
                                name: "HOOYAD_ENDPOINT".to_string(),
                                // Route this proxy to a specific node
                                value: Some(format!("hooyad-node-{target_node_index}-service.{namespace}.svc.cluster.local:8531")),
                                ..Default::default()
                            },
                            k8s_openapi::api::core::v1::EnvVar {
                                name: "HOOYA_WEB_PROXY_LOG_LEVEL".to_string(),
                                value: Some("debug".to_string()),
                                ..Default::default()
                            },
                        ]),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            },
            ..Default::default()
        }),
        ..Default::default()
    };

    let deployments: Api<Deployment> =
        Api::namespaced(client.clone(), namespace);
    deployments
        .create(&PostParams::default(), &deployment)
        .await
        .context("Failed to create proxy deployment")?;

    debug!("Created proxy deployment: {}", proxy_index);
    Ok(())
}

async fn create_proxy_service(
    client: &Client,
    namespace: &str,
    proxy_index: u32,
) -> Result<()> {
    let mut labels = BTreeMap::new();
    labels.insert("app".to_string(), format!("hooya-web-proxy-{proxy_index}"));

    let service = Service {
        metadata: k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta {
            name: Some(format!("hooya-web-proxy-{proxy_index}-service")),
            namespace: Some(namespace.to_string()),
            ..Default::default()
        },
        spec: Some(ServiceSpec {
            selector: Some(labels),
            ports: Some(vec![ServicePort {
                name: Some("http".to_string()),
                port: 8532,
                target_port: Some(IntOrString::Int(8532)),
                ..Default::default()
            }]),
            ..Default::default()
        }),
        ..Default::default()
    };

    let services: Api<Service> = Api::namespaced(client.clone(), namespace);
    services
        .create(&PostParams::default(), &service)
        .await
        .context("Failed to create proxy service")?;

    debug!("Created proxy service: {}", proxy_index);
    Ok(())
}

async fn wait_for_deployments_ready(
    client: &Client,
    namespace: &str,
    nodes: &[NodeConfig],
) -> Result<()> {
    let deployments: Api<Deployment> =
        Api::namespaced(client.clone(), namespace);

    for node in nodes {
        let deployment_name = format!("hooyad-{}", node.name);
        info!("Waiting for deployment {} to be ready...", deployment_name);

        for _ in 0..60 {
            // Wait up to 5 minutes
            if let Ok(deployment) = deployments.get(&deployment_name).await {
                if let Some(status) = deployment.status {
                    if let (Some(ready), Some(replicas)) =
                        (status.ready_replicas, status.replicas)
                    {
                        if ready == replicas && ready > 0 {
                            info!("Deployment {} is ready", deployment_name);
                            break;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    Ok(())
}

async fn wait_for_proxy_deployments_ready(
    client: &Client,
    namespace: &str,
    proxy_count: u32,
) -> Result<()> {
    let deployments: Api<Deployment> =
        Api::namespaced(client.clone(), namespace);

    for i in 0..proxy_count {
        let deployment_name = format!("hooya-web-proxy-{i}");
        info!("Waiting for proxy {} to be ready...", deployment_name);

        for _ in 0..60 {
            // Wait up to 5 minutes
            if let Ok(deployment) = deployments.get(&deployment_name).await {
                if let Some(status) = deployment.status {
                    if let (Some(ready), Some(replicas)) =
                        (status.ready_replicas, status.replicas)
                    {
                        if ready == replicas && ready > 0 {
                            info!(
                                "Proxy deployment {} is ready",
                                deployment_name
                            );
                            break;
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    }

    Ok(())
}
