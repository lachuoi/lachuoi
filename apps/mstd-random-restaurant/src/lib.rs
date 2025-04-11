use anyhow::anyhow;
use rand::seq::SliceRandom;
use rand::{distr::Alphanumeric, Rng};
use serde_json::Value;
use spin_cron_sdk::{cron_component, Metadata};
use spin_sdk::http::{Method::Get, Method::Post, Request, Response};
use spin_sdk::variables;
use std::str;

#[derive(Debug, Default)]
struct Place {
    name: String,
    lat: f64,
    lng: f64,
    place_id: String,
    address: String,
    rating: f64,
    photos: Vec<Photo>,
}

#[derive(Debug, Default)]
struct Photo {
    reference: String,
    content_disposition: Option<String>,
    content_length: Option<i32>,
    content_type: Option<String>,
    bytes: Box<Vec<u8>>,
    owner: Option<String>,
    description: Option<String>,
    mstd_mediaid: Option<i64>,
}

#[derive(Debug, Clone)]
struct Geopoint {
    lat: f64,
    lng: f64,
    country: String,
    population: Option<i64>,
}

#[cron_component]
async fn handle_cron_event(_: Metadata) -> anyhow::Result<()> {
    let mut place: Place = Place::default();

    let _ = loop {
        let locations = random_place().await?;
        let location = locations[0].to_owned();
        if let Some(p) = near_by_search(location, &mut place).await? {
            break p;
        }
        std::thread::sleep(std::time::Duration::from_millis(2_500));
    };
    get_place_details(&mut place).await?;
    get_images(&mut place).await?;
    get_image_descriptions(&mut place).await?;

    println!("------");
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

async fn near_by_search(
    geopoint: Geopoint,
    place: &mut Place,
) -> anyhow::Result<Option<usize>> {
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

    let filtered_place = filtered_places[0].to_owned();

    place.name = filtered_place
        .get("name")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    place.lat = filtered_place
        .get("geometry")
        .unwrap()
        .get("location")
        .unwrap()
        .get("lat")
        .unwrap()
        .as_f64()
        .unwrap();
    place.lng = filtered_place
        .get("geometry")
        .unwrap()
        .get("location")
        .unwrap()
        .get("lng")
        .unwrap()
        .as_f64()
        .unwrap();
    place.place_id = filtered_place
        .get("place_id")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    place.rating = filtered_place.get("rating").unwrap().as_f64().unwrap();

    Ok(Some(filtered_places.len()))
}
async fn get_place_details(place: &mut Place) -> anyhow::Result<()> {
    // Get restaurnat's detailed photos and formatted_address

    let api_key = variables::get("google_location_api_key")
        .expect("You must set the SPIN_VARIABLE_MSTD_RANDOM_RESTAURANT_GOOGLE_LOCATION_API_KEY in  environment var!");

    let api_uri: String = format!(
        "https://maps.googleapis.com/maps/api/place/details/json?place_id={}&fields=photos,formatted_address&key={}",
        place.place_id, api_key
    );

    let request = Request::builder().method(Get).uri(api_uri).build();
    let response: Response = spin_sdk::http::send(request).await?;

    let a = str::from_utf8(response.body()).unwrap();
    let b: Value = serde_json::from_str(a).unwrap();

    place.address = b["result"]["formatted_address"].to_string();

    for i in 0..4 {
        let aa = b["result"]["photos"][i]["photo_reference"].to_owned();
        if !aa.is_null() {
            let mut photo: Photo = Photo::default();
            photo.reference = aa.as_str().unwrap().to_owned();
            place.photos.push(photo);
        }
    }
    Ok(())
}

const MAX_HTTP_REDIRECTION: usize = 5;

async fn fetch_until_200(mut uri: String) -> anyhow::Result<Response> {
    for _ in 0..MAX_HTTP_REDIRECTION {
        let req = Request::builder().method(Get).uri(uri.clone()).build();
        let res: Response = spin_sdk::http::send(req).await?;

        match res.status() {
            &302u16 => {
                if let Some(location) = res.header("location") {
                    uri = location
                        .as_str()
                        .ok_or_else(|| {
                            anyhow!("Invalid 'Location' header encoding")
                        })?
                        .to_string();
                } else {
                    return Err(anyhow!(
                        "302 response without 'Location' header"
                    ));
                }
            }
            &200u16 => return Ok(res),
            &404u16 => return Ok(res),
            status => {
                return Err(anyhow!("Unexpected status code: {}", status))
            }
        }
    }
    Err(anyhow!(
        "Too many redirects (exceeded {MAX_HTTP_REDIRECTION})"
    ))
}

async fn get_images(place: &mut Place) -> anyhow::Result<()> {
    let api_key = variables::get("google_location_api_key")
        .expect("You must set the SPIN_VARIABLE_MSTD_RANDOM_RESTAURANT_GOOGLE_LOCATION_API_KEY in  environment var!");
    for photo in &mut place.photos {
        let aa = &photo.reference;
        let image_uri = format!(
                "https://maps.googleapis.com/maps/api/place/photo?maxwidth=1080&photoreference={}&key={}",
                aa, api_key
            );
        let res: Response = fetch_until_200(image_uri).await?;
        let content_length: i32 = res
            .header("content-length")
            .unwrap()
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        let content_type = res
            .header("content-type")
            .unwrap()
            .as_str()
            .unwrap_or_default();
        let content_disposition =
            res.header("content-disposition").unwrap().as_str().unwrap();
        let img_bytes = res.body().to_vec();
        photo.content_length = Some(content_length);
        photo.content_type = Some(content_type.to_string());
        photo.content_disposition = Some(content_disposition.to_string());
        photo.bytes = Box::new(img_bytes);
    }
    Ok(())
}

async fn extract_filename(header: &str) -> anyhow::Result<Option<String>> {
    Ok(header
        .split(';')
        .find(|part| part.trim_start().starts_with("filename="))
        .and_then(|part| {
            part.trim()
                .strip_prefix("filename=")?
                .trim_matches('"')
                .to_owned()
                .into()
        }))
}

async fn build_multipart_body(
    photo: &mut Photo,
) -> anyhow::Result<(String, Vec<u8>)> {
    // Generate a random boundary string
    let boundary: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();

    // Construct the multipart form body
    let mut body = Vec::new();

    // Add the opening boundary
    body.extend_from_slice(format!("--{}\r\n", boundary).as_bytes());

    // Add content disposition (assuming a file upload)
    body.extend_from_slice(b"Content-Disposition: form-data; name=\"file\"; filename=\"upload.bin\"\r\n");
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");

    // Add the file content
    body.extend_from_slice(photo.bytes.as_ref());

    // Add the closing boundary
    body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

    Ok((boundary, body))
}

async fn get_image_descriptions(place: &mut Place) -> anyhow::Result<()> {
    for photo in &mut place.photos {
        // let content_length = photo.content_length;
        println!("$$$$$$$$$$");

        let (boundary, body) = build_multipart_body(photo).await?;

        let content_length = body.len().to_string();

        let request = Request::builder()
            .method(Post)
            .uri("http://localhost:3000/image/description")
            //.uri("https://seungjin.requestcatcher.com/foo2")
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={}", boundary),
            )
            .header("Content-Length", content_length)
            .body(body)
            .build();
        let response: Response = spin_sdk::http::send(request).await?;
        println!("{}", response.status());
        println!("{:?}", str::from_utf8(response.body()));
    }
    Ok(())
}
