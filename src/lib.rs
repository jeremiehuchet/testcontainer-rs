use container::{GenericContainer, GenericContainerBuilder};

pub mod container;
pub mod docker_client;
pub mod image;

pub async fn postgresql() -> GenericContainerBuilder {
    GenericContainer::from_image("postgres:latest")
        .add_env("POSTGRES_DB", "test")
        .add_env("POSTGRES_USER", "test")
        .add_env("POSTGRES_PASSWORD", "test")
        .add_exposed_tcp_port(5432)
        .with_command(&["postgres", "-c", "fsync=off"])
        .wait_for_log_on_startup(r".*database system is ready to accept connections.*\s")
}
