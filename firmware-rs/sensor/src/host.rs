use std::time::Duration;

use anyhow::Context;
use rumqttc::{AsyncClient, MqttOptions, QoS};
use tracing::{info, warn};

use thermostat_common::{TOPIC_SENSOR_HUMIDITY, TOPIC_SENSOR_STATUS, TOPIC_SENSOR_TEMP};

pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mqtt_host = std::env::var("MQTT_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let mqtt_port = std::env::var("MQTT_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(1883);

    let mut mqtt_options = MqttOptions::new("thermostat-sensor-rust", mqtt_host, mqtt_port);

    if let Ok(user) = std::env::var("MQTT_USER") {
        let pass = std::env::var("MQTT_PASS").unwrap_or_default();
        mqtt_options.set_credentials(user, pass);
    }

    let (mqtt, mut eventloop) = AsyncClient::new(mqtt_options, 32);

    mqtt.publish(TOPIC_SENSOR_STATUS, QoS::AtLeastOnce, true, "online")
        .await
        .context("failed to publish sensor online status")?;

    tokio::spawn(async move {
        loop {
            if let Err(err) = eventloop.poll().await {
                warn!("sensor mqtt poll error: {err}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    });

    info!("sensor publisher started");

    let mut tick: u64 = 0;
    let mut interval = tokio::time::interval(Duration::from_secs(30));

    loop {
        interval.tick().await;
        tick = tick.saturating_add(1);

        // Hardware integration point:
        // replace these simulated readings with DS18B20 + DHT11 drivers on ESP target.
        let temperature_f = 68.0 + ((tick % 8) as f32 * 0.2);
        let humidity = 42.0 + ((tick % 6) as f32 * 0.5);

        let temp_payload = format!("{temperature_f:.1}");
        let humidity_payload = format!("{humidity:.1}");

        mqtt.publish(TOPIC_SENSOR_TEMP, QoS::AtLeastOnce, true, temp_payload)
            .await
            .context("failed to publish sensor temperature")?;
        mqtt.publish(
            TOPIC_SENSOR_HUMIDITY,
            QoS::AtLeastOnce,
            true,
            humidity_payload,
        )
        .await
        .context("failed to publish sensor humidity")?;
    }
}
