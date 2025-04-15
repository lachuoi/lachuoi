use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use rss::{Channel, Item};
use spin_cron_sdk::{cron_component, Metadata};
use spin_sdk::{
    http::{Method::Get, Method::Post, Request, Response},
    sqlite::{Connection, Value as SqlValue},
    variables,
};
use std::str::{self};

#[cron_component]
async fn handle_cron_event(_: Metadata) -> anyhow::Result<()> {
    println!("Newspenguin RSS starting");

    if check_process_lock().unwrap().is_some() {
        println!("Newspenguin process lock exist - exit");
        println!("Newspenguin RSS finished");
        return Ok(());
    }

    process_lock()?;

    let channel = get_rss().await.unwrap();

    let rss_last_build_date = NaiveDateTime::parse_from_str(
        channel.last_build_date().unwrap(),
        "%Y-%m-%d %H:%M:%S",
    )
    .expect("Newspenguin Failed to parse date");

    let recorded_last_build_date = last_build_date().await?;

    if recorded_last_build_date.is_none() {
        update_last_build_date(rss_last_build_date).await?;
        return Ok(());
    }

    if rss_last_build_date > recorded_last_build_date.unwrap() {
        let new_items =
            get_new_items(channel, recorded_last_build_date.unwrap()).await?;
        post_to_mastodon(new_items).await?;
        update_last_build_date(rss_last_build_date).await?;
    } else {
        update_last_build_date(rss_last_build_date).await?;
    }

    println!("Newspenguin RSS finished");

    process_unlock()?;

    Ok(())
}

const DB_KEY_LAST_BUILD: &str = "newspenguin-rss.last_build_date";
const DB_KEY_LOCK: &str = "newspenguin-rss.lock";

async fn get_rss() -> anyhow::Result<Channel> {
    let rss_uri = variables::get("rss_uri").unwrap();
    let request = Request::builder().method(Get).uri(rss_uri).build();
    let response: Response = spin_sdk::http::send(request).await?;

    if response.status() != &200u16 {
        println!("NOT 200");
    }

    let rss = str::from_utf8(response.body()).unwrap().as_bytes();
    let channel = Channel::read_from(rss)?;

    Ok(channel)
}

async fn last_build_date() -> anyhow::Result<Option<NaiveDateTime>> {
    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");

    let execute_params = [SqlValue::Text(DB_KEY_LAST_BUILD.to_string())];
    let rowset = connection.execute(
        "SELECT value FROM kv_store WHERE key = ?",
        execute_params.as_slice(),
    )?;

    if rowset.rows().count() == 0 {
        return Ok(None);
    }

    let a = rowset.rows.first().unwrap();
    match a.get::<&str>(0) {
        Some(a) => {
            let naive_dt =
                NaiveDateTime::parse_from_str(a, "%Y-%m-%d %H:%M:%S")
                    .expect("Failed to parse date");
            Ok(Some(naive_dt))
        }
        None => Ok(None),
    }
}

async fn update_last_build_date(d: NaiveDateTime) -> anyhow::Result<()> {
    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");
    let execute_params = [
        SqlValue::Text(d.to_string()),
        SqlValue::Text(DB_KEY_LAST_BUILD.to_string()),
    ];
    let rowset = connection
        .execute(
            "UPDATE kv_store SET value = ? WHERE key = ?",
            execute_params.as_slice(),
        )
        .unwrap();

    // https://github.com/spinframework/spin/issues/3092
    // if rowset.rows().count() == 0 {
    //     let execute_params = [
    //         SqlValue::Text(NAME.to_string()),
    //         SqlValue::Text(d.to_string()),
    //     ];
    //     let rowset = connection.execute(
    //         "INSERT INTO last_build_date (NAME, LAST_BUILD_DATE) VALUES (?,?)",
    //         execute_params.as_slice(),
    //     );
    // }
    // {
    //     // DELETE FROM last_build_date WHERE rowid NOT IN ( SELECT MAX(rowid) FROM last_build_date WHERE name = "newspenguin");
    //     let execute_params = [SqlValue::Text(NAME.to_string())];
    //     let rowset = connection.execute("DELETE FROM last_build_date WHERE rowid NOT IN ( SELECT MAX(rowid) FROM last_build_date WHERE name = ?)", execute_params.as_slice());
    // }

    Ok(())
}

async fn get_new_items(
    channel: Channel,
    dt: NaiveDateTime,
) -> anyhow::Result<Vec<Item>> {
    let mut new_items: Vec<Item> = Vec::new();
    for item in channel.items() {
        let item_pub_date = NaiveDateTime::parse_from_str(
            item.pub_date().unwrap(),
            "%Y-%m-%d %H:%M:%S",
        )
        .expect("Failed to parse date");
        if dt < item_pub_date {
            new_items.push(item.clone());
        }
    }
    new_items.reverse();
    Ok(new_items)
}

async fn post_to_mastodon(msgs: Vec<Item>) -> anyhow::Result<()> {
    let mstd_api_uri = format!(
        "{}/api/v1/statuses",
        variables::get("mstd_api_uri").unwrap()
    );
    let mstd_access_token = variables::get("mstd_access_token").unwrap();

    if msgs.is_empty() {
        println!("Newspenguin RSS - Nothing to publish");
        return Ok(());
    }

    for item in msgs {
        let msg: String = format!(
            "{}:\n{}\n{}\n({})",
            item.title.clone().unwrap(),
            item.description.unwrap(),
            item.link.unwrap(),
            item.pub_date.unwrap()
        );
        let form_body = format!("status={}&visibility={}", &msg, "public");
        let request = Request::builder()
            .method(Post)
            .uri(&mstd_api_uri)
            .header("AUTHORIZATION", format!("Bearer {}", mstd_access_token))
            .body(form_body)
            .build();
        let response: Response = spin_sdk::http::send(request).await?;

        if response.status().to_owned() == 200u16 {
            println!("Rss published: [{}]", item.title.unwrap());
        }
    }

    Ok(())
}

fn check_process_lock() -> anyhow::Result<Option<()>> {
    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");

    let execute_params = [SqlValue::Text(DB_KEY_LOCK.to_string())];
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
            .expect("Newspenguin Failed to parse datetime");

    // Assume it's already in UTC (you can adjust here if it's in local time or another zone)
    let utc_dt: DateTime<Utc> =
        DateTime::<Utc>::from_naive_utc_and_offset(naive_dt, Utc);

    let now = Utc::now();
    let one_hour_ago = now - Duration::minutes(5);

    if utc_dt < one_hour_ago {
        println!("Newspenguin lock process is older than 5 min. unlock it.");
        process_unlock()?; // Unlock process that is older than 5 min.
        return Ok(None);
    };

    Ok(Some(()))
}

fn process_lock() -> anyhow::Result<()> {
    println!("Newspenguin process lock");
    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");
    let execute_params = [SqlValue::Text(DB_KEY_LOCK.to_string())];
    let rowset = connection.execute(
        "INSERT INTO kv_store (key,value) VALUES (?, NULL)",
        execute_params.as_slice(),
    )?;
    Ok(())
}

fn process_unlock() -> anyhow::Result<()> {
    println!("Newspenguin process unlock");
    let connection =
        Connection::open("lachuoi").expect("lachuoi db connection error");
    let execute_params = [SqlValue::Text(DB_KEY_LOCK.to_string())];
    let rowset = connection.execute(
        "DELETE FROM kv_store WHERE key = ?",
        execute_params.as_slice(),
    )?;
    Ok(())
}
