use crate::{
    docker_client::{ContainerClient, DockerClient},
    image::DockerImage,
};
use docker_api::opts::{ContainerCreateOpts, HostPort, PublishPort};
use log::info;
use regex::Regex;
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct GenericContainerBuilder {
    image: DockerImage,
    environment_variables: HashMap<String, String>,
    exposed_ports: HashMap<String, Option<u16>>,
    volumes: HashSet<String>,
    labels: HashMap<String, String>,
    command: Option<Vec<String>>,
    wait_strategy_on_startup: ReadyStrategy,
    start_timeout: Duration,
}

impl GenericContainerBuilder {
    fn new(image: DockerImage) -> Self {
        GenericContainerBuilder {
            image,
            environment_variables: HashMap::new(),
            exposed_ports: HashMap::new(),
            volumes: HashSet::new(),
            labels: HashMap::new(),
            command: None,
            wait_strategy_on_startup: ReadyStrategy::None,
            start_timeout: Duration::from_secs(30),
        }
    }

    pub fn add_env<S: Into<String>>(mut self, key: S, value: S) -> Self {
        self.environment_variables.insert(key.into(), value.into());
        self
    }

    pub fn add_exposed_port<S: Into<String>>(
        mut self,
        host_port: Option<u16>,
        container_port_spec: S,
    ) -> Self {
        self.exposed_ports
            .insert(container_port_spec.into(), host_port);
        self
    }

    pub fn add_exposed_tcp_port(self, port: u16) -> Self {
        self.add_exposed_port(None, format!("{port}/tcp"))
    }

    pub fn add_fixed_exposed_tcp_port(self, host_port: u16, container_port: u16) -> Self {
        self.add_exposed_port(Some(host_port), format!("{container_port}/tcp"))
    }

    pub fn add_volume<S: Into<String>>(mut self, volume: S) -> Self {
        self.volumes.insert(volume.into());
        self
    }

    pub fn add_label<S: Into<String>>(mut self, name: S, value: S) -> Self {
        self.labels.insert(name.into(), value.into());
        self
    }

    pub fn with_command(mut self, command_parts: &[&str]) -> Self {
        self.command = Some(command_parts.iter().map(|s| s.to_string()).collect());
        self
    }

    pub fn wait_for_log_on_startup<S: Into<String>>(mut self, log_regex: S) -> Self {
        let regex: String = log_regex.into();
        let regex = regex
            .parse()
            .expect(format!("a valid regular expression but it was {regex}").as_str());
        self.wait_strategy_on_startup = ReadyStrategy::LogMessageRegExp(regex);
        self
    }

    pub fn with_start_timeout(mut self, duration_expression: &str) -> Self {
        let duration = parse_duration::parse(duration_expression)
            .expect(format!("a parseable duration but it was {duration_expression}").as_str());
        self.start_timeout = duration;
        self
    }

    pub async fn create(self) -> Result<GenericContainer, docker_api::Error> {
        let docker = DockerClient::default();
        if let Some(build_opts) = self.image.clone().into() {
            info!("🐋 Building image {}", self.image);
            docker.build(&build_opts).await?;
        } else if !docker.image_exists_locally(&self.image).await? {
            info!("🐋 Pulling image {}", self.image);
            docker.pull(&self.image).await?
        }
        let container = docker.create(&self.clone().into()).await?;
        Ok(GenericContainer {
            params: self,
            container,
        })
    }
}

impl Into<ContainerCreateOpts> for GenericContainerBuilder {
    fn into(self) -> ContainerCreateOpts {
        let mut opts = ContainerCreateOpts::builder()
            .image(self.image.to_string())
            .env(
                self.environment_variables
                    .iter()
                    .map(|(name, value)| format!("{name}={value}")),
            )
            .labels(self.labels)
            .volumes(self.volumes)
            .publish_all_ports();

        if let Some(command) = self.command {
            opts = opts.command(command);
        }

        for (exposed_port, host_port) in self.exposed_ports {
            opts = if let Some(host_port) = host_port {
                opts.expose(
                    exposed_port.parse().unwrap(),
                    HostPort::new(host_port.into()),
                )
            } else {
                opts.publish(exposed_port.parse().unwrap())
            }
        }

        opts.build()
    }
}

#[derive(Clone)]
pub enum ReadyStrategy {
    LogMessageRegExp(Regex),
    StateHealthy,
    None,
}

impl ReadyStrategy {
    pub(crate) async fn wait(
        &self,
        container: &ContainerClient,
        timeout: Duration,
    ) -> Result<(), docker_api::Error> {
        let timeout_instant = Instant::now() + timeout;
        loop {
            match self {
                ReadyStrategy::LogMessageRegExp(regex) => {
                    let logs = container.logs().await?;
                    if regex.is_match(&logs) {
                        return Ok(());
                    }
                }
                ReadyStrategy::StateHealthy => {
                    if let Some(health_state) = container.health_state().await? {
                        if health_state == "".to_string() {
                            return Ok(());
                        }
                    }
                }
                ReadyStrategy::None => return Ok(()),
            }
            if timeout_instant < Instant::now() {
                break;
            } else {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
            }
        }
        return Err(docker_api::Error::StringError(
            "Container takes too much time to be ready".to_string(),
        ));
    }
}

pub struct GenericContainer {
    params: GenericContainerBuilder,
    container: ContainerClient,
}

impl GenericContainer {
    pub fn from_image<S: Into<DockerImage>>(full_image_name: S) -> GenericContainerBuilder {
        GenericContainerBuilder::new(full_image_name.into())
    }

    pub async fn start(&self) -> Result<(), docker_api::Error> {
        self.container
            .start_and_wait(
                &self.params.wait_strategy_on_startup,
                self.params.start_timeout,
            )
            .await?;
        Ok(())
    }

    pub async fn stop(&self) -> Result<(), docker_api::Error> {
        self.container.stop().await
    }

    pub async fn kill(&self) -> Result<(), docker_api::Error> {
        self.container.kill().await
    }

    pub fn get_host_port<S: Into<String>>(&self, container_port_spec: S) -> Option<u16> {
        let container_port_spec: String = container_port_spec.into();
        let ro_state = self.container.running_state.read().unwrap();
        ro_state.as_ref()?.ports.get(&container_port_spec).copied()
    }
}
