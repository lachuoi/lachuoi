use anyhow::Result;
use rand::distr::weighted::WeightedIndex;
use rand::prelude::*;
use serde_hjson;
use serde_json::{json, Number, Value};
use spin_sdk::{
    http::{IntoResponse, Params, Request, Response, Router, Method::Get},
    http_component,
    key_value::Store,
    sqlite::{Connection, QueryResult, Value as SqlValue},
};
use std::str;

/// A simple Spin HTTP component.

#[http_component]
async fn handle_root(req: Request) -> Result<impl IntoResponse> {
    let mut router = Router::new();
    router.get_async("/place/random/weighted", weighted_random_location);
    router.get_async(
        "/place/random/weighted/population",
        weighted_random_location,
    );
    router.get_async("/place/random", weighted_random_location);
    Ok(router.handle(req))
}

#[allow(dead_code)]
async fn random_location(
    _req: Request,
    _params: Params,
) -> anyhow::Result<impl IntoResponse> {
    let connection =
        Connection::open("geoname").expect("geoname libsql connection error");

    let execute_params = [SqlValue::Integer(50000)];
    let rowset = connection.execute(
        "SELECT geonameid, alternatenames, asciiname, country, elevation, fclass, latitude, longitude, moddate, name, population, timezone FROM cities15000 WHERE population >= ? ORDER BY RANDOM() LIMIT 1",
        execute_params.as_slice(),
    )?;

    Ok(Response::builder()
        .status(200)
        .header("content-type", "application/json")
        .body(query_result_to_json(&rowset))
        .build())
}

const CACHEKEY: &str = "city-pop-pair";

async fn weighted_random_location(
    _req: Request,
    _params: Params,
) -> Result<Response> {
    // https://docs.rs/rand/latest/rand/distr/weighted/struct.WeightedIndex.html
    let cache = Store::open("mem")?;

    // Getting weighted factor
    let request = Request::builder()
        .method(Get)
        .uri("https://raw.githubusercontent.com/seungjin/lachuoi/refs/heads/main/assets/random-place-wegith.hjson")
        
        .build();
    let response: Response = spin_sdk::http::send(request).await?;
    let response_body = str::from_utf8(response.body()).unwrap();

    let weighted_factors: Value = serde_hjson::from_str(response_body).unwrap();

    // TODO: receive this over param
    // let weighted_factors = json!({
    //     "base_population" : 50000,
    //     "country" : {
    //         // Filtering out. China and North Korea
    //         "CN": 0, "KP": 0,
    //         // Weighted more
    //         "DE": 2.5, "GB": 2, "FR": 2.5, "ES": 2, "IT": 2.5, "TW": 1.5, "TH": 2,
    //         "NL": 2, "PT": 1.8,
    //         // Weighted less
    //         "IN": 0.25, "ID": 0.4, "PK": 0.4
    //     },
    //     "city": {
    //         "Bangkok": 0.8
    //      }
    // });

    let a = match cache.get(CACHEKEY)? {
        Some(x) => {
            println!("Cache retrived");
            x
        }
        None => {
            println!("Writing to cache");
            let default_pop = &Value::Number(Number::from_i128(49999).unwrap());
            let base_population = weighted_factors
                .get("base_population")
                .unwrap_or(default_pop);

            let connection = Connection::open("geoname")
                .expect("geoname libsql connection error");
            let execute_params =
                [SqlValue::Integer(base_population.as_i64().unwrap())];
            let rowset = connection.execute(
                "SELECT geonameid, population, country, asciiname FROM cities15000 WHERE population >= ? ",
                execute_params.as_slice(),
            );
            let rows = rowset.unwrap().rows;

            let mut cities_points: Vec<(i64, f64)> = Vec::new();
            let weighted_countries = weighted_factors.get("country").unwrap();
            let weighted_cities = weighted_factors.get("city").unwrap();
            for row in rows {
                let population = row
                    .get::<i64>(1)
                    .map(|v| v as f64)
                    .expect("Expected a float but found another type!");

                // Weighted by Countries
                if let Some(obj) = weighted_countries.as_object() {
                    for (key, val) in obj.iter() {
                        if row.get::<&str>(2).unwrap() == key {
                            let weighted_point =
                                population * val.as_f64().unwrap();
                            cities_points.push((
                                row.get::<i64>(0).unwrap(),
                                weighted_point,
                            ));
                        }
                    }
                } else {
                    cities_points.push((row.get(0).unwrap(), population));
                }

                // Weighted by Cities
                if let Some(obj) = weighted_cities.as_object() {
                    for (key, val) in obj.iter() {
                        if row.get::<&str>(3).unwrap() == key {
                            let weighted_point =
                                population * val.as_f64().unwrap();
                            cities_points.push((
                                row.get::<i64>(0).unwrap(),
                                weighted_point,
                            ));
                        }
                    }
                } else {
                    cities_points.push((row.get(0).unwrap(), population));
                }
            }

            let json_str = serde_json::to_vec(&cities_points).unwrap();

            let cache = Store::open("mem")?;
            cache.set(CACHEKEY, json_str.as_slice())?;
            json_str
        }
    };

    let b = serde_json::from_slice::<Value>(&a).unwrap();

    let data: Vec<(i64, f64)> = serde_json::from_value(b).unwrap();
    let data2 = data.as_slice();
    //let data = serde_json::to_string(&b).unwrap();
    let mut rng = rand::rng();
    let dist = WeightedIndex::new(data2.iter().map(|item| item.1)).unwrap();
    let random_index = dist.sample(&mut rng);

    let id = data[random_index].0;
    let value = data[random_index].1;
    // println!("{} -- {} - {}", random_index, id, value);

    let connection =
        Connection::open("geoname").expect("geoname libsql connection error");
    let execute_params = [SqlValue::Integer(id as i64)];
    let rowset = connection.execute(
        "SELECT geonameid, alternatenames, asciiname, country, elevation, fclass, latitude, longitude, moddate, name, population, timezone FROM cities15000 WHERE geonameid = ?",
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
