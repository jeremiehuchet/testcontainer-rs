use crate::{container::ReadyStrategy, image::DockerImage};
use docker_api::{
    models::{ContainerInspect200Response, ImageBuildChunk, NetworkSettings},
    opts::{
        ContainerCreateOpts, ContainerStopOpts, ImageBuildOpts, ImageFilter, ImageListOpts,
        LogsOpts, PullOpts,
    },
    Container, Docker,
};
use futures_util::StreamExt;
use log::{debug, error};
use std::{collections::HashMap, fmt::Display, sync::RwLock, time::Duration};

pub(crate) struct DockerClient {
    docker: Docker,
}

impl Default for DockerClient {
    fn default() -> Self {
        Self {
            docker: Docker::unix("/var/run/docker.sock"),
        }
    }
}

impl DockerClient {
    pub(crate) async fn image_exists_locally(
        &self,
        image: &DockerImage,
    ) -> Result<bool, docker_api::Error> {
        let images = self
            .docker
            .images()
            .list(
                &ImageListOpts::builder()
                    .filter(vec![image.clone().into()])
                    .build(),
            )
            .await?;
        Ok(!images.is_empty())
    }

    pub(crate) async fn pull(&self, image: &DockerImage) -> Result<(), docker_api::Error> {
        let images = self.docker.images();
        let mut stream = images.pull(&PullOpts::builder().image(image.get_full_name()).build());
        while let Some(build_chunk) = stream.next().await {
            match build_chunk {
                Ok(build_chunk) => debug!("{}", Loggable::from(build_chunk)),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) async fn build(&self, build_opts: &ImageBuildOpts) -> Result<(), docker_api::Error> {
        let images = self.docker.images();
        let mut stream = images.build(&build_opts);
        while let Some(build_chunk) = stream.next().await {
            match build_chunk {
                Ok(build_chunk) => debug!("{}", Loggable::from(build_chunk)),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub(crate) async fn create(
        &self,
        opts: &ContainerCreateOpts,
    ) -> Result<ContainerClient, docker_api::Error> {
        Ok(ContainerClient::new(
            self.docker.containers().create(&opts).await?,
        ))
    }
}

struct Loggable {
    message: String,
}

impl From<ImageBuildChunk> for Loggable {
    fn from(chunk: ImageBuildChunk) -> Self {
        let message = match chunk {
            ImageBuildChunk::Update { stream } => format!("Update: {stream}"),
            ImageBuildChunk::Error {
                error,
                error_detail,
            } => format!("Error: {error}: {}", error_detail.message),
            ImageBuildChunk::Digest { aux } => format!("Digest: {}", aux.id),
            ImageBuildChunk::PullStatus {
                status,
                id,
                progress,
                progress_detail: _,
            } => {
                let id = id.unwrap_or("".to_string());
                let progress = progress.unwrap_or("".to_string());
                format!("Pull: {status} {id} {progress}")
            }
        };
        Loggable { message }
    }
}

impl Display for Loggable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = self.message.trim();
        write!(f, "🐋 {message}")
    }
}

pub(crate) struct ContainerClient {
    inner_container: Container,
    pub(crate) running_state: RwLock<Option<RunningState>>,
}

impl ContainerClient {
    fn new(container: Container) -> Self {
        ContainerClient {
            inner_container: container,
            running_state: RwLock::new(None),
        }
    }

    pub async fn health_state(&self) -> Result<Option<String>, docker_api::Error> {
        let inspect = self.inner_container.inspect().await?;
        Ok(inspect.state.and_then(|state| state.health?.status))
    }

    pub async fn logs(&self) -> Result<String, docker_api::Error> {
        let opts = LogsOpts::builder().stdout(true).stderr(true).all().build();
        let logs = self
            .inner_container
            .logs(&opts)
            .map(|chunk| match chunk {
                Ok(chunk) => chunk.to_vec(),
                Err(e) => {
                    error!("🐋 Error: {e}");
                    vec![]
                }
            })
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        Ok(String::from_utf8_lossy(&logs).to_string())
    }

    pub(crate) async fn start_and_wait(
        &self,
        ready_strategy: &ReadyStrategy,
        timeout: Duration,
    ) -> Result<(), docker_api::Error> {
        self.inner_container.start().await?;
        ready_strategy.wait(self, timeout).await?;
        let mut rw_state = self.running_state.write().unwrap();
        let inspect = self.inner_container.inspect().await?;
        let running_state: RunningState = inspect.into();
        println!("🐋 Container {} is ready", running_state.name);
        *rw_state = Some(running_state);
        Ok(())
    }

    pub(crate) async fn stop(&self) -> Result<(), docker_api::Error> {
        self.inner_container
            .stop(&ContainerStopOpts::builder().build())
            .await?;
        let mut rw_state = self.running_state.write().unwrap();
        let name = rw_state.clone().map_or("???".to_string(), |s| s.name);
        *rw_state = None;
        println!("🐋 Container {} is stopped", &name);
        Ok(())
    }

    pub(crate) async fn kill(&self) -> Result<(), docker_api::Error> {
        self.inner_container
            .stop(&ContainerStopOpts::builder().signal("SIGKILL").build())
            .await?;
        let mut rw_state = self.running_state.write().unwrap();
        let name = rw_state.clone().map_or("???".to_string(), |s| s.name);
        *rw_state = None;
        println!("🐋 Container {} killed", &name);
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct RunningState {
    id: String,
    name: String,
    pub(crate) ports: HashMap<String, u16>,
}
impl From<ContainerInspect200Response> for RunningState {
    fn from(inspect: ContainerInspect200Response) -> Self {
        let ports = Self::extract_port_mapping(inspect.network_settings).unwrap_or(HashMap::new());

        RunningState {
            id: inspect.id.expect("container should have an id"),
            name: inspect.name.expect("container should have a name"),
            ports,
        }
    }
}
impl RunningState {
    fn extract_port_mapping(
        network_settings: Option<NetworkSettings>,
    ) -> Option<HashMap<String, u16>> {
        let ports: HashMap<String, u16> = network_settings?
            .ports?
            .iter()
            .filter_map(|(container_port_spec, host_ports)| match host_ports {
                Some(host_ports) => {
                    let triples: Vec<_> = host_ports
                        .iter()
                        .filter_map(|port_binding| {
                            let port_binding = port_binding.clone();
                            match (port_binding.host_ip, port_binding.host_port) {
                                (Some(host_ip), Some(host_port)) => {
                                    Some((container_port_spec, host_ip, host_port))
                                }
                                (_, _) => None,
                            }
                        })
                        .collect();
                    Some(triples)
                }
                None => None,
            })
            .flatten()
            .filter_map(|(container_port_spec, host_ip, host_port)| {
                if host_ip == "0.0.0.0".to_string() {
                    Some((container_port_spec.into(), host_port.parse().unwrap()))
                } else {
                    None
                }
            })
            .collect();
        Some(ports)
    }
}
