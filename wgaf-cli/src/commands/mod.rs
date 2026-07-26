use zbus::Connection;

pub async fn ping() -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::session().await?;
    let reply = connection
        .call_method(
            Some(wgaf_common::BUS_NAME),
            wgaf_common::OBJECT_PATH,
            Some(wgaf_common::INTERFACE_NAME),
            "Ping",
            &(),
        )
        .await?;
    let response: String = reply.body().deserialize()?;
    println!("{response}");
    Ok(())
}
