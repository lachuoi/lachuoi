use rand;
use rand::seq::SliceRandom;
use serde_json::Value;
use spin_cron_sdk::{cron_component, Metadata};
use spin_sdk::http::{Method::Get, Request, Response};
use spin_sdk::variables;
use std::str;

#[cron_component]
async fn handle_cron_event(_: Metadata) -> anyhow::Result<()> {
    let near_place = loop {
        let locations = random_place().await?;
        let location = locations[0].to_owned();
        if let Some(place) = near_by_search(location).await? {
            break place;
        }
        std::thread::sleep(std::time::Duration::from_millis(1_000));
    };

    Ok(())
}

async fn random_place() -> anyhow::Result<Vec<Geopoint>> {
    let request = Request::builder()
        .method(Get)
        .uri("http://localhost:3000/random_place")
        // .uri("http://random-place.spin.internal")
        .build();
    let response: Response = spin_sdk::http::send(request)
        .await
        .expect("random-place internal service call failed");
    let response_body = str::from_utf8(response.body()).unwrap();

    let mut locations: Vec<Geopoint> = Vec::new();
    for location in serde_json::from_str::<Vec<Value>>(response_body).unwrap() {
        let geopoint = Geopoint {
            lat: location.get("latitude").unwrap().as_f64().unwrap(),
            lng: location.get("longitude").unwrap().as_f64().unwrap(),
            country: location
                .get("country")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
            population: location.get("population").unwrap().as_i64(),
        };
        locations.push(geopoint);
    }

    Ok(locations)
}

#[derive(Debug, Default)]
struct Place {
    name: String,
    lat: f64,
    lng: f64,
    place_id: String,
    address: String,
    rating: f64,
    pics: Vec<String>,
    pics_tmp_dir: String,
    mstd_media_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
struct Geopoint {
    lat: f64,
    lng: f64,
    country: String,
    population: Option<i64>,
}

async fn near_by_search(geopoint: Geopoint) -> anyhow::Result<Option<Place>> {
    let api_key = variables::get("google_location_api_key")
        .expect("You must set the SPIN_VARIABLE_MSTD_RANDOM_RESTAURANT_GOOGLE_LOCATION_API_KEY in  environment var!");
    let api_url: String = format!(
        "https://maps.googleapis.com/maps/api/place/nearbysearch/json?location={}%2C{}&radius=100000&type=cafe&keyword=coffee&key={}",
        geopoint.lat, geopoint.lng, api_key
    );

    let request = Request::builder().method(Get).uri(api_url).build();
    let response: Response = spin_sdk::http::send(request).await?;

    let response_body: Value =
        serde_json::from_str(str::from_utf8(response.body()).unwrap()).unwrap();

    let mut filtered_places: Vec<Value> = Vec::new();
    for i in response_body["results"].as_array().unwrap() {
        if i["types"]
            .as_array()
            .unwrap()
            .contains(&Value::String("hotel".to_string()))
            || i["types"]
                .as_array()
                .unwrap()
                .contains(&Value::String("lodge".to_string()))
            || i["types"]
                .as_array()
                .unwrap()
                .contains(&Value::String("lodging".to_string()))
            || i["types"]
                .as_array()
                .unwrap()
                .contains(&Value::String("gas_station".to_string()))
            || i["types"]
                .as_array()
                .unwrap()
                .contains(&Value::String("convenience_store".to_string()))
            || i["types"]
                .as_array()
                .unwrap()
                .contains(&Value::String("grocery_or_supermarket".to_string()))
            || i["types"]
                .as_array()
                .unwrap()
                .contains(&Value::String("night_club".to_string()))
        {
            continue;
        }
        if i["rating"].as_f64().unwrap_or(0_f64) >= 3_f64
            && i["user_ratings_total"].as_f64().unwrap_or(0_f64) >= 100_f64
        {
            filtered_places.push(i.clone());
        }
    }

    // let p = filtered_places.choose(&mut rand::thread_rng()).unwrap();

    if filtered_places.is_empty() {
        return Ok(None);
    }

    let mut rng = rand::rng();
    filtered_places.shuffle(&mut rng);

    println!("{}", filtered_places[0]);

    // name: filtered_places[0].get("name").unwrap().to_string(),
    // lat: filtered_places[0].get("geometry").unwrap().get("location").unwrap().get("lat").unwrap().as_f64().unwrap(),
    // lng: filtered_places[0].get("geometry").unwrap().get("location").unwrap().get("lng").unwrap().as_f64().unwrap(),

    // place_id: filtered_places[0].get("place_id").unwrap().to_string(),

    Ok(None)

    // r.place_id = p.clone()["place_id"].as_str().unwrap().to_string();
    // r.name = p.clone()["name"].as_str().unwrap().to_string();
    // if p.get("rating").is_some() {
    //     r.rating = p["rating"].as_f64().unwrap();
    // } else {
    //     r.rating = 0.0;
    // };
}
async fn get_place_details() -> anyhow::Result<()> {
    // Get restaurnat's detailed photos and formatted_address

    let place_id = "ChIJyfDnfQlHWBQR2Z74Us5KFxk";
    let api_key = variables::get("google_location_api_key")
        .expect("You must set the SPIN_VARIABLE_MSTD_RANDOM_RESTAURANT_GOOGLE_LOCATION_API_KEY in  environment var!");
    let api_url: String = format!(
        "https://maps.googleapis.com/maps/api/place/details/json?place_id={}&fields=photos&formatted_address&key={}",
        place_id, api_key
    );

    let request = Request::builder().method(Get).uri(api_url).build();
    let response: Response = spin_sdk::http::send(request).await?;

    //println!("{:#?}", resp);

    //println!("{:#?}", resp["result"]["photos"]);

    Ok(())
}
