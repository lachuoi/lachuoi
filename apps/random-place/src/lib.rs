use anyhow::Result;
use rand::distr::weighted::WeightedIndex;
use rand::prelude::*;
use serde_json::{json, Value};
use spin_sdk::{
    http::{IntoResponse, Params, Request, Response, Router},
    http_component,
    key_value::Store,
    sqlite::{Connection, QueryResult, Value as SqlValue},
};

/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    let mut router = Router::new();
    router.get("/random-place/weighted", weighted_random_location);
    router.get(
        "/random-place/weighted/population",
        weighted_random_location,
    );
    router.any("/random-place", random_location);
    Ok(router.handle(req))

    // Ok(Response::builder()
    //     .status(200)
    //     .header("content-type", "plain/text")
    //     .body("arsarsars")
    //     .build())
}

fn random_location(
    _req: Request,
    _params: Params,
) -> anyhow::Result<impl IntoResponse> {
    let connection =
        Connection::open("geoname").expect("geoname libsql connection error");

    let execute_params = [SqlValue::Integer(50000)];
    let rowset = connection.execute(
        "SELECT alternatenames, asciiname, country, elevation, fclass, latitude, longitude, moddate, name, population, timezone FROM cities15000 WHERE population >= ? ORDER BY RANDOM() LIMIT 1",
        execute_params.as_slice(),
    )?;

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(query_result_to_json(&rowset))
        .build())
}

const CACHEKEY: &str = "city-pop-pair";

fn weighted_random_location(
    _req: Request,
    _params: Params,
) -> Result<Response> {
    // https://docs.rs/rand/latest/rand/distr/weighted/struct.WeightedIndex.html
    let cache = Store::open("mem")?;

    // TODO: receive this over param
    let weighted_factors = json!({
        "country" : {
            "DE": 3, "GB": 3, "FR": 3, "ES": 3, "IT": 3, "TW": 3, "TH": 3,
            "MX": 3, "PT": 3, "CN": 0, "IN": 0.5, "ID": 0.7, "PK": 0.7
        }
    });

    let a = match cache.get(CACHEKEY)? {
        Some(x) => {
            println!("Cache retrived");
            String::from_utf8(x).unwrap()
        }
        None => {
            println!("Writing to cache");
            let connection = Connection::open("geoname")
                .expect("geoname libsql connection error");
            let execute_params = [SqlValue::Integer(50_000)];
            let rowset = connection.execute(
                "SELECT rowid, population, country FROM cities15000 WHERE population >= ?",
                execute_params.as_slice(),
            );
            let rows = rowset.unwrap().rows;
            // let cities_population: Vec<(u64, u64)> = rows
            //     .iter()
            //     .map(|r| (r.get::<u64>(0).unwrap(), r.get::<u64>(1).unwrap()))
            //     .collect();

            let weighted_country = weighted_factors.get("country").unwrap();
            let mut cities_population: Vec<(i64, f64)> = Vec::new();
            for row in rows {
                let population =
                    row.get::<i64>(1).map(|v| v as f64).unwrap_or_else(|| {
                        panic!("Expected a float but found another type!");
                    });

                if let Some(obj) = weighted_country.as_object() {
                    for (key, val) in obj.iter() {
                        let factor = val.as_f64().unwrap();
                        if row.get::<&str>(2).unwrap() == key {
                            cities_population.push((
                                row.get(0).unwrap(),
                                population * factor,
                            ))
                        } else {
                            cities_population
                                .push((row.get(0).unwrap(), population));
                        }
                    }
                }
            }

            let json_str = serde_json::to_string(&cities_population).unwrap();

            let cache = Store::open("mem")?;
            cache.set(CACHEKEY, json_str.as_bytes())?;
            json_str
        }
    };

    let data: Vec<(u64, f64)> = serde_json::from_str(a.as_str()).unwrap();
    let mut rng = rand::rng();
    let dist =
        WeightedIndex::new(data.iter().map(|item| item.1 as f64)).unwrap();
    let random_index = dist.sample(&mut rng);

    let &(id, _value) = data.get(random_index).unwrap();

    let connection =
        Connection::open("geoname").expect("geoname libsql connection error");
    let execute_params = [SqlValue::Integer(id as i64)];
    let rowset = connection.execute(
        "SELECT alternatenames, asciiname, country, elevation, fclass, latitude, longitude, moddate, name, population, timezone FROM cities15000 WHERE rowid = ?",
        execute_params.as_slice(),
    )?;

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(query_result_to_json(&rowset))
        .build())
}

fn query_result_to_json(query_result: &QueryResult) -> String {
    let rows_json: Vec<Value> = query_result
        .rows
        .iter()
        .map(|row| {
            let obj = query_result
                .columns
                .iter()
                .zip(&row.values)
                .map(|(col, val)| {
                    let json_val = match val {
                        SqlValue::Integer(i) => json!(i),
                        SqlValue::Real(f) => json!(f),
                        SqlValue::Text(s) => json!(s),
                        SqlValue::Blob(_) => json!(null), // Blob not supported here
                        SqlValue::Null => json!(null),
                    };
                    (col.clone(), json_val)
                })
                .collect::<serde_json::Map<_, _>>();
            Value::Object(obj)
        })
        .collect();

    let result = json!(rows_json);
    serde_json::to_string_pretty(&result).unwrap()
}
