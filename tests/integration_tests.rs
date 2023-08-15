use docker_api::Error;
use testcontainers::postgresql;
use tokio_postgres::NoTls;

#[tokio::test]
async fn should_create_postgresql_container() -> Result<(), Error> {
    let container = postgresql().await.create().await?;
    container.start().await?;
    container.stop().await?;
    Ok(())
}

#[tokio::test]
async fn should_expose_postgresql_ports() -> Result<(), Error> {
    let container = postgresql().await.create().await?;
    container.start().await?;

    let port = container.get_host_port("5432/tcp").unwrap();

    let params = format!("host=localhost port={port} dbname=test user=test password=test");
    let (client, conn) = tokio_postgres::connect(&params, NoTls).await.unwrap();
    tokio::spawn(conn);
    let databases: Vec<String> = client
        .query("SELECT datname FROM pg_database ORDER BY datname", &[])
        .await
        .unwrap()
        .iter()
        .map(|row| row.get("datname"))
        .collect();

    assert_eq!(databases, &["postgres", "template0", "template1", "test"]);

    container.kill().await?;
    Ok(())
}
