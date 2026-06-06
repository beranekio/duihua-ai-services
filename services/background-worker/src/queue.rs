use std::env;

use anyhow::{Context, Result};
use duihua_common::response_store_from_env;
use redis::{
    aio::MultiplexedConnection,
    streams::{StreamAutoClaimOptions, StreamAutoClaimReply, StreamId, StreamReadOptions},
    AsyncCommands, RedisError,
};

use crate::worker;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueueMessage {
    pub stream_id: String,
    pub response_id: String,
}

pub struct QueueConfig {
    pub redis_url: String,
    pub stream_key: String,
    pub consumer_group: String,
    pub consumer_name: String,
    pub block_ms: usize,
    pub autoclaim_min_idle_ms: usize,
    pub autoclaim_batch_size: usize,
}

impl QueueConfig {
    pub fn from_env() -> Result<Self> {
        let redis_url =
            env::var("RESPONSE_ID_STORE_URL").unwrap_or_else(|_| "redis://valkey:6379".to_string());
        let stream_key = env::var("BACKGROUND_QUEUE_STREAM_KEY")
            .unwrap_or_else(|_| "duihua:responses:background".to_string());
        let consumer_group = env::var("BACKGROUND_QUEUE_CONSUMER_GROUP")
            .unwrap_or_else(|_| "duihua-background".to_string());
        let consumer_name = env::var("BACKGROUND_QUEUE_CONSUMER_NAME")
            .or_else(|_| env::var("HOSTNAME"))
            .unwrap_or_else(|_| "duihua-background-worker".to_string());
        let block_ms = env::var("BACKGROUND_QUEUE_BLOCK_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(5_000);
        let autoclaim_min_idle_ms = env::var("BACKGROUND_QUEUE_AUTOCLAIM_MIN_IDLE_MS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(60_000);
        let autoclaim_batch_size = env::var("BACKGROUND_QUEUE_AUTOCLAIM_BATCH_SIZE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(10);

        Ok(Self {
            redis_url,
            stream_key,
            consumer_group,
            consumer_name,
            block_ms,
            autoclaim_min_idle_ms,
            autoclaim_batch_size,
        })
    }
}

pub async fn run() -> Result<()> {
    let config = QueueConfig::from_env()?;
    let response_store = response_store_from_env().await?;
    let client = redis::Client::open(config.redis_url.as_str())
        .with_context(|| format!("invalid RESPONSE_ID_STORE_URL {}", config.redis_url))?;
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .with_context(|| "failed to connect to background queue")?;

    ensure_consumer_group(&mut connection, &config).await?;
    let mut autoclaim_cursor = "0-0".to_string();

    loop {
        let autoclaim: StreamAutoClaimReply = connection
            .xautoclaim_options(
                &config.stream_key,
                &config.consumer_group,
                &config.consumer_name,
                config.autoclaim_min_idle_ms,
                &autoclaim_cursor,
                StreamAutoClaimOptions::default().count(config.autoclaim_batch_size),
            )
            .await?;
        autoclaim_cursor = autoclaim.next_stream_id;
        for message in messages_from_stream_ids(&autoclaim.claimed) {
            handle_message(&mut connection, &config, &response_store, message).await?;
        }

        let pending_opts = StreamReadOptions::default()
            .group(&config.consumer_group, &config.consumer_name)
            .count(config.autoclaim_batch_size);
        if let Some(reply) = connection
            .xread_options(&[&config.stream_key], &["0"], &pending_opts)
            .await?
        {
            for message in messages_from_read_reply(&reply) {
                handle_message(&mut connection, &config, &response_store, message).await?;
            }
        }

        let new_opts = StreamReadOptions::default()
            .group(&config.consumer_group, &config.consumer_name)
            .block(config.block_ms)
            .count(1);
        if let Some(reply) = connection
            .xread_options(&[&config.stream_key], &[">"], &new_opts)
            .await?
        {
            for message in messages_from_read_reply(&reply) {
                handle_message(&mut connection, &config, &response_store, message).await?;
            }
        }
    }
}

async fn ensure_consumer_group(
    connection: &mut MultiplexedConnection,
    config: &QueueConfig,
) -> Result<()> {
    match connection
        .xgroup_create_mkstream(&config.stream_key, &config.consumer_group, "0")
        .await
    {
        Ok(()) => Ok(()),
        Err(err) if is_busygroup(&err) => Ok(()),
        Err(err) => Err(err).context("failed to create background queue consumer group"),
    }
}

async fn handle_message(
    connection: &mut MultiplexedConnection,
    config: &QueueConfig,
    response_store: &duihua_common::ResponseStore,
    message: QueueMessage,
) -> Result<()> {
    worker::process_response(response_store, &message.response_id).await?;
    let _: usize = connection
        .xack(
            &config.stream_key,
            &config.consumer_group,
            &[message.stream_id.as_str()],
        )
        .await?;
    let _: usize = connection
        .xdel(&config.stream_key, &[message.stream_id.as_str()])
        .await?;
    Ok(())
}

pub fn response_id_from_stream_entry(entry: &StreamId) -> Option<String> {
    entry.get("response_id")
}

pub fn queue_message_from_stream_entry(entry: &StreamId) -> Option<QueueMessage> {
    response_id_from_stream_entry(entry).map(|response_id| QueueMessage {
        stream_id: entry.id.clone(),
        response_id,
    })
}

pub fn messages_from_stream_ids(entries: &[StreamId]) -> Vec<QueueMessage> {
    entries
        .iter()
        .filter_map(queue_message_from_stream_entry)
        .collect()
}

pub fn messages_from_read_reply(reply: &redis::streams::StreamReadReply) -> Vec<QueueMessage> {
    reply
        .keys
        .iter()
        .flat_map(|key| key.ids.iter())
        .filter_map(queue_message_from_stream_entry)
        .collect()
}

fn is_busygroup(err: &RedisError) -> bool {
    err.code() == Some("BUSYGROUP")
}

#[cfg(test)]
mod tests {
    use super::*;
    use redis::FromRedisValue;
    use redis::Value;

    #[test]
    fn extracts_response_id_from_stream_entry() {
        let entry = StreamId {
            id: "1717670000000-0".to_string(),
            map: [(
                "response_id".to_string(),
                Value::BulkString(b"resp_abc".to_vec()),
            )]
            .into(),
            milliseconds_elapsed_from_delivery: None,
            delivered_count: None,
        };

        assert_eq!(
            response_id_from_stream_entry(&entry).as_deref(),
            Some("resp_abc")
        );
        assert_eq!(
            queue_message_from_stream_entry(&entry),
            Some(QueueMessage {
                stream_id: "1717670000000-0".to_string(),
                response_id: "resp_abc".to_string(),
            })
        );
    }

    #[test]
    fn ignores_stream_entries_without_response_id() {
        let entry = StreamId {
            id: "1717670000000-0".to_string(),
            map: [("other".to_string(), Value::BulkString(b"x".to_vec()))].into(),
            milliseconds_elapsed_from_delivery: None,
            delivered_count: None,
        };

        assert!(queue_message_from_stream_entry(&entry).is_none());
    }

    #[test]
    fn parses_messages_from_xreadgroup_reply() {
        let value = Value::Array(vec![Value::Array(vec![
            Value::BulkString(b"duihua:responses:background".to_vec()),
            Value::Array(vec![Value::Array(vec![
                Value::BulkString(b"1717670000000-0".to_vec()),
                Value::Array(vec![
                    Value::BulkString(b"response_id".to_vec()),
                    Value::BulkString(b"resp_xyz".to_vec()),
                ]),
            ])]),
        ])]);
        let reply = redis::streams::StreamReadReply::from_redis_value(value).expect("reply");
        assert_eq!(
            messages_from_read_reply(&reply),
            vec![QueueMessage {
                stream_id: "1717670000000-0".to_string(),
                response_id: "resp_xyz".to_string(),
            }]
        );
    }
}
