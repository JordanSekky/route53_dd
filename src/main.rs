use std::{net::IpAddr, path::PathBuf, time::Duration};

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
    fs::File,
    io::AsyncReadExt,
    task::JoinSet,
    time::{self},
};

shadow!(build);

#[derive(Parser, Debug)]
#[command(about, long_about = None)]
struct Args {
    #[arg(long, default_value_t = false)]
    daemon: bool,

    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    #[arg(long, short, action)]
    version: bool,
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

#[derive(Deserialize, Clone, Debug)]
struct ConfigFile {
    zones: Vec<HostedZoneConfig>,
}

#[allow(clippy::const_is_empty)]
fn print_version() {
    if !build::TAG.is_empty() {
        if !build::GIT_CLEAN {
            println!("{}-dirty", build::TAG);
        } else {
            println!("{}", build::TAG);
        }
    } else if !build::LAST_TAG.is_empty() {
        println!("{}", build::LAST_TAG);
    } else if !build::GIT_CLEAN {
        println!("{}-{}-dirty", build::PKG_VERSION, build::SHORT_COMMIT);
    } else {
        println!("{}-{}", build::PKG_VERSION, build::SHORT_COMMIT);
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    SimpleLogger::new()
        .with_level(log::LevelFilter::Info)
        .init()
        .unwrap();

    let args = Args::parse();

    if args.version {
        print_version();
        return Ok(());
    }

    let mut config_file_string = String::new();
    info!("Loading config file from {}", args.config.to_string_lossy());
    File::open(args.config)
        .await?
        .read_to_string(&mut config_file_string)
        .await?;
    let config_file: ConfigFile = toml::from_str(&config_file_string)?;
    let mut task_set = JoinSet::new();

    for zone in config_file.zones {
        task_set.spawn(daemon_update_zone(zone, args.daemon));
    }
    let results = task_set.join_all().await;

    results.into_iter().collect()
}

async fn daemon_update_zone(zone: HostedZoneConfig, daemon: bool) -> Result<(), Error> {
    if !daemon {
        if let Err(e) = update_hosted_zone(zone.clone()).await {
            error!("Error while updating zone {:?}: {:?}", zone, e);
            return Err(e);
        }
        return Ok(());
    }
    let mut interval = time::interval(Duration::from_secs(60 * zone.update_frequency_minutes));
    loop {
        interval.tick().await;
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
    info!("Found hosted zones {:?}", hosted_zones);
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
