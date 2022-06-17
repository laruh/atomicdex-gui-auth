use super::*;
use async_trait::async_trait;
use bytes::Buf;
use ctx::AppConfig;
use db::Db;
use hyper::{header, Body, Request, Response, StatusCode};
use redis::FromRedisValue;
use serde::{Deserialize, Serialize};

pub(crate) const DB_STATUS_LIST: &str = "status_list";

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub(crate) struct IpStatusPayload {
    pub(crate) ip: String,
    pub(crate) status: i8,
}

pub(crate) async fn post_ip_status(
    cfg: &AppConfig,
    req: Request<Body>,
) -> GenericResult<Response<Body>> {
    let whole_body = hyper::body::aggregate(req).await?;
    let payload: Vec<IpStatusPayload> = serde_json::from_reader(whole_body.reader())?;

    let mut db = Db::create_instance(cfg).await;
    db.bulk_insert_ip_status(payload).await?;

    Ok(Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(Vec::new()))?)
}

pub(crate) async fn get_ip_status_list(cfg: &AppConfig) -> GenericResult<Response<Body>> {
    let mut db = Db::create_instance(cfg).await;
    let list = db.read_ip_status_list().await;

    let list: Vec<IpStatusPayload> = list
        .iter()
        .map(|v| IpStatusPayload {
            ip: v.0.clone(),
            status: v.1,
        })
        .collect();
    let serialized = serde_json::to_string(&list)?;

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serialized))?)
}

#[derive(Debug, Deserialize, PartialEq)]
pub(crate) enum IpStatus {
    /// Follow the normal procedure.
    None = -1,
    /// Means incoming request will be respond as `403 Forbidden`.
    Trusted,
    /// Means incoming request will bypass the security checks on the middleware layer.
    Blocked,
}

impl IpStatus {
    pub(crate) fn from_i8(value: i8) -> Self {
        match value {
            0 => Self::Trusted,
            1 => Self::Blocked,
            _ => Self::None,
        }
    }
}

impl FromRedisValue for IpStatus {
    fn from_redis_value(v: &redis::Value) -> redis::RedisResult<Self> {
        let val: i8 = redis::from_redis_value(v)?;

        Ok(IpStatus::from_i8(val))
    }
}

#[async_trait]
pub(crate) trait IpStatusOperations {
    async fn insert_ip_status(&mut self, ip: String, status: IpStatus) -> GenericResult<()>;
    async fn bulk_insert_ip_status(&mut self, payload: Vec<IpStatusPayload>) -> GenericResult<()>;
    async fn read_ip_status(&mut self, ip: String) -> IpStatus;
    async fn read_ip_status_list(&mut self) -> Vec<(String, i8)>;
}

#[async_trait]
impl IpStatusOperations for Db {
    async fn insert_ip_status(&mut self, ip: String, status: IpStatus) -> GenericResult<()> {
        Ok(redis::cmd("HSET")
            .arg(DB_STATUS_LIST)
            .arg(&[ip, format!("{}", status as i8)])
            .query_async(&mut self.connection)
            .await?)
    }

    async fn bulk_insert_ip_status(&mut self, payload: Vec<IpStatusPayload>) -> GenericResult<()> {
        let mut pipe = redis::pipe();
        let formatted: Vec<(String, i8)> =
            payload.iter().map(|v| (v.ip.clone(), v.status)).collect();
        pipe.hset_multiple(DB_STATUS_LIST, &formatted);
        pipe.query_async(&mut self.connection).await?;

        Ok(())
    }

    async fn read_ip_status(&mut self, ip: String) -> IpStatus {
        redis::cmd("HGET")
            .arg(DB_STATUS_LIST)
            .arg(ip)
            .query_async(&mut self.connection)
            .await
            .unwrap_or(IpStatus::None)
    }

    async fn read_ip_status_list(&mut self) -> Vec<(String, i8)> {
        redis::cmd("HGETALL")
            .arg(DB_STATUS_LIST)
            .query_async(&mut self.connection)
            .await
            .unwrap_or_default()
    }
}

#[test]
fn test_ip_status_constants() {
    assert_eq!(DB_STATUS_LIST, "status_list");
}

#[test]
fn test_ip_status_serialzation_and_deserialization() {
    let json_ip_status = serde_json::json!({
        "ip": "127.0.0.1",
        "status": 0
    });

    let actual_ip_status: IpStatusPayload =
        serde_json::from_str(&json_ip_status.to_string()).unwrap();

    let expected_ip_status = IpStatusPayload {
        ip: String::from("127.0.0.1"),
        status: 0,
    };

    assert_eq!(actual_ip_status, expected_ip_status);

    // Backwards
    let json = serde_json::to_value(expected_ip_status).unwrap();
    assert_eq!(json_ip_status, json);
    assert_eq!(json_ip_status.to_string(), json.to_string());
}

#[test]
fn test_if_ip_status_values_same_as_before() {
    assert_eq!(IpStatus::None, IpStatus::from_i8(-1));
    assert_eq!(IpStatus::Trusted, IpStatus::from_i8(0));
    assert_eq!(IpStatus::Blocked, IpStatus::from_i8(1));
}

#[test]
fn test_from_redis_value() {
    let redis_val = redis::Value::Int(-1);
    let val: IpStatus = redis::from_redis_value(&redis_val).unwrap();
    assert_eq!(val, IpStatus::None);

    let redis_val = redis::Value::Int(0);
    let val: IpStatus = redis::from_redis_value(&redis_val).unwrap();
    assert_eq!(val, IpStatus::Trusted);

    let redis_val = redis::Value::Int(1);
    let val: IpStatus = redis::from_redis_value(&redis_val).unwrap();
    assert_eq!(val, IpStatus::Blocked);
}
