use std::{net::IpAddr, time::Duration};

use anyhow::{anyhow, Error};
use aws_config::{self, BehaviorVersion, Region};
use aws_sdk_route53::{
    types::{Change, ChangeBatch, ResourceRecord, ResourceRecordSet},
    Client,
};
use credential_provider::AwsCredentials;
use log::{error, info};
use serde::Deserialize;
mod credential_provider;
use clap::Parser;
use shadow_rs::shadow;
use simple_logger::SimpleLogger;
use tokio::{
    select,
    time::{self},
};
use tokio_util::sync::CancellationToken;

shadow!(build);

#[derive(Parser, Debug)]
#[command(about, long_about = None, version = version())]
struct Args {
    #[arg(long, short, default_value_t = false)]
    daemon: bool,

    #[arg(long, short, env = "UPDATE_FREQUENCY_MINUTES", default_value_t = 5)]
    update_frequency_minutes: u64,

    #[arg(long, env = "ZONE_NAME")]
    zone_name: String,

    #[arg(long, env = "RECORD_NAME")]
    record_name: String,

    #[arg(long, env = "IPV4", default_value_t = true)]
    ipv4: bool,

    #[arg(long, env = "IPV6", default_value_t = false)]
    ipv6: bool,

    #[arg(long, env = "AWS_REGION")]
    region: String,

    #[arg(long, env = "AWS_ACCESS_KEY_ID")]
    aws_access_key_id: String,

    #[arg(long, env = "AWS_SECRET_ACCESS_KEY")]
    aws_secret_access_key: String,

    #[arg(long, env = "AWS_SESSION_TOKEN")]
    aws_session_token: Option<String>,

    #[arg(long, env = "TTL_SECONDS", default_value_t = 300)]
    ttl_seconds: i64,
}

#[derive(Deserialize, Clone, Debug)]
struct HostedZoneConfig {
    pub update_frequency_minutes: u64,
    pub zone_name: String,
    pub record_name: String,
    pub ipv4: bool,
    pub ipv6: bool,
    pub region: String,
    pub aws_credentials: AwsCredentials,
    pub ttl_seconds: i64,
}

#[allow(clippy::const_is_empty)]
fn version() -> &'static str {
    let s = if build::GIT_CLEAN {
        format!("{}-{}", build::PKG_VERSION, build::SHORT_COMMIT)
    } else {
        format!("{}-{}-dirty", build::PKG_VERSION, build::SHORT_COMMIT)
    };
    Box::leak(s.into_boxed_str())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .init()
        .unwrap();

    let args = Args::parse();

    let zone = HostedZoneConfig {
        update_frequency_minutes: args.update_frequency_minutes,
        zone_name: args.zone_name,
        record_name: args.record_name,
        ipv4: args.ipv4,
        ipv6: args.ipv6,
        region: args.region,
        aws_credentials: AwsCredentials {
            access_key_id: args.aws_access_key_id,
            secret_access_key: args.aws_secret_access_key,
            session_token: args.aws_session_token,
            expires_after: None,
        },
        ttl_seconds: args.ttl_seconds,
    };

    let shutdown_token = tokio_util::sync::CancellationToken::new();
    let cloned_token = shutdown_token.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.unwrap();
        info!("Shutting down.");
        cloned_token.cancel();
    });

    daemon_update_zone(zone, args.daemon, shutdown_token).await
}

async fn daemon_update_zone(
    zone: HostedZoneConfig,
    daemon: bool,
    shutdown_token: CancellationToken,
) -> Result<(), Error> {
    if !daemon {
        if let Err(e) = update_hosted_zone(zone.clone()).await {
            error!("Error while updating zone {:?}: {:?}", zone, e);
            return Err(e);
        }
        return Ok(());
    }
    let mut interval = time::interval(Duration::from_secs(60 * zone.update_frequency_minutes));
    loop {
        select! {
            _ = interval.tick() => {}
            _ = shutdown_token.cancelled() => {
                info!("{}.{} shutdown.", zone.record_name, zone.zone_name);
                break Ok(())
            }
        }
        if let Err(e) = update_hosted_zone(zone.clone()).await {
            error!("Error while updating zone {:?}: {:?}", zone, e);
            error!("Trying again at {:?}", interval.period())
        } else {
            info!("Updating again at {:?}", interval.period())
        };
    }
}

async fn update_hosted_zone(zone: HostedZoneConfig) -> Result<(), Error> {
    info!("Updating hosted zone {:?}", &zone);
    let config = aws_config::defaults(BehaviorVersion::latest())
        .credentials_provider(zone.aws_credentials.clone())
        .region(Region::new(zone.region.clone()))
        .load()
        .await;
    let client = Client::new(&config);

    let hosted_zones = client
        .list_hosted_zones_by_name()
        .dns_name(zone.zone_name.clone())
        .send()
        .await?;
    let hosted_zone = hosted_zones
        .hosted_zones
        .into_iter()
        .find(|_| true)
        .ok_or(anyhow!("No hosted zone found."))?
        .id;
    info!("Found hosted zone id {}", hosted_zone);

    let mut record_changes: Vec<Change> = Vec::with_capacity(2);

    if zone.ipv4 {
        let web_client_ipv4 = reqwest::Client::builder()
            .local_address("0.0.0.0".parse::<IpAddr>()?)
            .build()?;
        let ipv4_result = web_client_ipv4.get("https://ifconfig.me/ip").send().await;
        let ipv4_result = ipv4_result?.error_for_status()?.text().await?;
        info!("Found ipv4 address: {:?}", ipv4_result);
        let ipv4: IpAddr = ipv4_result.parse()?;
        record_changes.push(
            Change::builder()
                .action(aws_sdk_route53::types::ChangeAction::Upsert)
                .resource_record_set(
                    ResourceRecordSet::builder()
                        .name(format!("{}.{}", zone.record_name, zone.zone_name))
                        .r#type(aws_sdk_route53::types::RrType::A)
                        .ttl(zone.ttl_seconds)
                        .resource_records(
                            ResourceRecord::builder().value(ipv4.to_string()).build()?,
                        )
                        .build()?,
                )
                .build()?,
        )
    }

    if zone.ipv6 {
        let web_client_ipv6 = reqwest::Client::builder()
            .local_address("::".parse::<IpAddr>()?)
            .build()?;

        let ipv6_result = web_client_ipv6.get("https://ifconfig.me/ip").send().await;
        let ipv6: IpAddr = ipv6_result?.error_for_status()?.text().await?.parse()?;
        info!("Found ipv6 address: {:?}", ipv6);
        record_changes.push(
            Change::builder()
                .action(aws_sdk_route53::types::ChangeAction::Upsert)
                .resource_record_set(
                    ResourceRecordSet::builder()
                        .name(format!("{}.{}", zone.record_name, zone.zone_name))
                        .r#type(aws_sdk_route53::types::RrType::Aaaa)
                        .ttl(zone.ttl_seconds)
                        .resource_records(
                            ResourceRecord::builder().value(ipv6.to_string()).build()?,
                        )
                        .build()?,
                )
                .build()?,
        )
    }

    if !record_changes.is_empty() {
        client
            .change_resource_record_sets()
            .hosted_zone_id(hosted_zone)
            .change_batch(
                ChangeBatch::builder()
                    .set_changes(Some(record_changes))
                    .build()?,
            )
            .send()
            .await?;
    }
    info!("Finished updating hosted zone {:?}", zone);

    Ok(())
}
