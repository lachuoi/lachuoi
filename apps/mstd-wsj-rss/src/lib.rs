use chrono::{self, DateTime, Duration, NaiveDateTime, Utc};
use convert_case::{Case, Casing};
use rss::{Channel, Item};
use serde_json::Value;
use spin_cron_sdk::{cron_component, Metadata};
use spin_sdk::{
    http::{Method::Get, Method::Post, Request, Response},
    sqlite::{Connection, Value as SqlValue},
    variables,
};
use std::str::{self};

#[cron_component]
async fn handle_cron_event(_: Metadata) -> anyhow::Result<()> {
    println!("WSJ RSS starting");

    let request = Request::builder()
        .method(Get)
        .uri("https://raw.githubusercontent.com/seungjin/lachuoi/refs/heads/main/assets/wsj-news-feeds.hjson")
        .build();
    let response: Response = spin_sdk::http::send(request).await?;
    let response_body = str::from_utf8(response.body()).unwrap();
    let wsj_rss_feeds: Value = serde_hjson::from_str(response_body).unwrap();

    if let Some(feeds) = wsj_rss_feeds.as_array() {
        for feed in feeds {
            // Extract the "name" and "url" from each feed
            if let (Some(name), Some(url)) = (
                feed.get("name").and_then(Value::as_str),
                feed.get("url").and_then(Value::as_str),
            ) {
                rss_eater(name.to_string(), url.to_string()).await?;
            }
        }
    }

    Ok(())
}

const DB_KEY_PREFIX: &str = "wsj-rss";

async fn rss_eater(name: String, url: String) -> anyhow::Result<()> {
    // let db_key_last_build = format!("{}.last_build_date", DB_KEY_PREFIX);

    if check_process_lock(&name).await.unwrap().is_some() {
        println!("WSJ {} process lock exist - exit", name);
        println!("WSJ RSS finished");
        return Ok(());
    }

    process_lock(&name).await?;

    let channel = get_rss(url).await.expect("Error from get_rss()");

    let rss_last_build_date = NaiveDateTime::parse_from_str(
        channel.last_build_date().unwrap(),
        "%a, %d %b %Y %H:%M:%S GMT",
    )
    .expect("WSJ Failed to parse date");

    let recorded_last_build_date =
        last_build_date(&name, rss_last_build_date).await?;

    if rss_last_build_date > recorded_last_build_date {
        let new_items =
            get_new_items(channel, recorded_last_build_date).await?;
        post_to_mastodon(&name, new_items).await?;
        update_last_build_date(&name, rss_last_build_date).await?;
    } else {
        update_last_build_date(&name, rss_last_build_date).await?;
    }

    process_unlock(&name).await?;
    println!("Mstd-Wsj RSS finished");

    Ok(())
}

async fn get_rss(rss_uri: String) -> anyhow::Result<Channel> {
    let request = Request::builder().method(Get).uri(&rss_uri).build();
    let response: Response = spin_sdk::http::send(request).await?;

    if response.status() != &200u16 {
        println!("Getting `{}` from `{}`", response.status(), rss_uri);
    }

    let rss = str::from_utf8(response.body()).unwrap().as_bytes();
    let channel = Channel::read_from(rss)?;

    Ok(channel)
}

async fn last_build_date(
    name: &String,
    dt: NaiveDateTime,
) -> anyhow::Result<NaiveDateTime> {
    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");

    let camel_name = name.to_case(Case::Camel);
    let db_key_last_build =
        format!("{}.{}.last_build_date", DB_KEY_PREFIX, camel_name);

    let execute_params = [SqlValue::Text(db_key_last_build)];
    let rowset = connection.execute(
        "SELECT value FROM kv_store WHERE key = ?",
        execute_params.as_slice(),
    )?;

    let a = rowset.rows;
    match !a.is_empty() {
        true => {
            let stored_dt = NaiveDateTime::parse_from_str(
                a.first().unwrap().get(0).unwrap(),
                "%Y-%m-%d %H:%M:%S",
            )
            .expect("Failed to parse date");
            Ok(stored_dt)
        }
        false => {
            let db_key_last_build =
                format!("{}.{}.last_build_date", DB_KEY_PREFIX, camel_name);
            let execute_params = [
                SqlValue::Text(db_key_last_build),
                SqlValue::Text(dt.to_string()),
            ];
            connection.execute(
                "INSERT INTO kv_store(KEY, VALUE) VALUEs(?,?)",
                execute_params.as_slice(),
            )?;

            Ok(dt)
        }
    }
}

async fn update_last_build_date(
    name: &String,
    d: NaiveDateTime,
) -> anyhow::Result<()> {
    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");

    let camel_name = name.to_case(Case::Camel);
    let db_key_last_build =
        format!("{}.{}.last_build_date", DB_KEY_PREFIX, camel_name);

    let execute_params = [
        SqlValue::Text(d.to_string()),
        SqlValue::Text(db_key_last_build),
    ];
    let _rowset = connection
        .execute(
            "UPDATE kv_store SET value = ? WHERE key = ?",
            execute_params.as_slice(),
        )
        .unwrap();

    Ok(())
}

async fn get_new_items(
    channel: Channel,
    recorded_last_build_date: NaiveDateTime,
) -> anyhow::Result<Vec<Item>> {
    let mut new_items: Vec<Item> = Vec::new();
    for item in channel.items() {
        let a = item.pub_date().unwrap();
        let item_pub_date =
            NaiveDateTime::parse_from_str(a, "%a, %d %b %Y %H:%M:%S GMT")
                .expect("Failed to parse date");
        if recorded_last_build_date < item_pub_date {
            new_items.push(item.clone());
        }
    }
    new_items.reverse();
    Ok(new_items)
}

async fn post_to_mastodon(
    name: &String,
    msgs: Vec<Item>,
) -> anyhow::Result<()> {
    let mstd_api_uri = format!(
        "{}/api/v1/statuses",
        variables::get("mstd_api_uri").unwrap()
    );
    let mstd_access_token = variables::get("mstd_access_token").unwrap();

    if msgs.is_empty() {
        println!("Mstd-Wsj RSS - Nothing to publish");
        return Ok(());
    }

    println!("POST TO MASTODON");
    for item in msgs {
        let msg: String = format!(
            "[{}] {}\n{} #WSJ\n{}\n({})",
            name,
            item.title.clone().unwrap_or("".to_string()),
            item.description.unwrap_or("".to_string()),
            item.link.unwrap_or("".to_string()),
            item.pub_date.unwrap_or("".to_string())
        )
        .trim()
        .to_string();
        let form_body = format!("status={}&visibility={}", &msg, "public");
        let request = Request::builder()
            .method(Post)
            .uri(&mstd_api_uri)
            .header("AUTHORIZATION", format!("Bearer {}", mstd_access_token))
            .body(form_body)
            .build();
        let response: Response = spin_sdk::http::send(request).await?;

        if response.status() == &200u16 {
            println!("WSJ-Rss published: {}", item.title.unwrap());
        }
    }

    Ok(())
}

async fn check_process_lock(name: &String) -> anyhow::Result<Option<()>> {
    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");

    let camel_name = name.to_case(Case::Camel);
    let db_key_last_build = format!("{}.{}.lock", DB_KEY_PREFIX, camel_name);

    let execute_params = [SqlValue::Text(db_key_last_build)];
    let rowset = connection.execute(
        "SELECT updated_at FROM kv_store WHERE key = ? ORDER BY updated_at DESC LIMIT 1",
        execute_params.as_slice(),
    )?;

    if rowset.rows().count() == 0 {
        return Ok(None);
    }

    let updated_at = rowset.rows[0].get::<&str>(0).unwrap();

    let naive_dt =
        NaiveDateTime::parse_from_str(updated_at, "%Y-%m-%d %H:%M:%S")
            .expect("Mstd-Wsj Failed to parse datetime");

    // Assume it's already in UTC (you can adjust here if it's in local time or another zone)
    let utc_dt: DateTime<Utc> =
        DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc);

    let now = Utc::now();
    let one_hour_ago = now - Duration::minutes(5);

    if utc_dt < one_hour_ago {
        println!("Mstd-Wsj lock process is older than 5 min. unlock it.");
        process_unlock(name).await?; // Unlock process that is older than 5 min.
        return Ok(None);
    };

    Ok(Some(()))
}

async fn process_lock(name: &String) -> anyhow::Result<()> {
    println!("Wsj-rss {} process lock", name);
    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");

    let camel_name = name.to_case(Case::Camel);
    let db_key_last_build = format!("{}.{}.lock", DB_KEY_PREFIX, camel_name);

    let execute_params = [SqlValue::Text(db_key_last_build)];
    let _rowset = connection.execute(
        "INSERT INTO kv_store (key,value) VALUES (?, NULL)",
        execute_params.as_slice(),
    )?;
    Ok(())
}

async fn process_unlock(name: &String) -> anyhow::Result<()> {
    let camel_name = name.to_case(Case::Camel);
    let db_key_last_build = format!("{}.{}.lock", DB_KEY_PREFIX, camel_name);

    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");
    let execute_params = [SqlValue::Text(db_key_last_build)];
    let _rowset = connection.execute(
        "DELETE FROM kv_store WHERE key = ?",
        execute_params.as_slice(),
    )?;
    Ok(())
}
