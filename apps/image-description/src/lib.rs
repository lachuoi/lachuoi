use anyhow::{bail, Result};
use base64::{engine::general_purpose, Engine as _};
use mime::Mime;
use multipart_2021::server::Multipart;
use serde_json::{json, Value};
use spin_sdk::{
    http::{IntoResponse, Method, Request, Response},
    http_component, variables,
};
use std::collections::HashMap;
use std::io::Read;
use std::str;
/// A simple Spin HTTP component.
#[http_component]
async fn handle_root(req: Request) -> anyhow::Result<impl IntoResponse> {
    println!("Handling request to {:?}", req.header("spin-full-url"));

    let mut _images: Vec<&[u8]> = Vec::new();

    if req.method() == &Method::Post {
        let body = req.body();
        let boundary = get_multipart_boundary(&req).unwrap(); // TODO: Match its error

        let mut mp: Multipart<&[u8]> = Multipart::with_body(body, boundary);

        while let Some(field) = mp.read_entry().unwrap() {
            let data: Result<Vec<u8>, std::io::Error> =
                field.data.bytes().collect();
            let a = data.unwrap();
            if let Some(desc) = image_description(a).await? {
                let desc_json = json!({
                    "description": desc
                });
                let json_string = serde_json::to_string(&desc_json).unwrap();
                return Ok(Response::builder()
                    .status(200)
                    .header("content-type", "application/json")
                    .body(json_string)
                    .build());
            }
        }
    }

    //println!("{:?}", str::from_utf8(req.body()).unwrap());

    Ok(Response::builder()
        .status(200)
        .header("content-type", "text/plain")
        .body("Hello World!")
        .build())
}

#[allow(dead_code)]
async fn image_description(
    image_bytes: Vec<u8>,
) -> anyhow::Result<Option<String>> {
    // curl -X POST \
    //  -H "Content-Type: application/json" \
    //  -H "x-goog-api-key: YOUR_API_KEY" \
    //  -H "x-goog-generativeai-api-version: v1beta" \
    //  -d @request.json \
    //  "https://generativelanguage.googleapis.com/v1beta/models/gemini-pro-vision:generateContent"

    let api_uri = variables::get("google_ai_api_uri").expect(
        "SPIN_VARIABLE_GOOGLE_AI_API_URI needed in environment variables",
    );
    let api_key = variables::get("google_ai_api_key")
        .expect("SPIN_VARIABLE_IMAGE_DESCRIPTION_GOOGLE_AI_API_KEY needed in environment variables");
    let prompt = variables::get("google_ai_prompt")
        .expect("SPIN_VARIABLE_IMAGE_DESCRIPTION_GOOGLE_AI_PROMPT needed in environment variables");

    let base64_image = encode_image_to_base64(image_bytes)?;

    let body = json!({
         "contents": [
            {
                "parts": [
                    {
                        "inlineData": {
                            "mimeType": "image/jpeg",
                            "data": base64_image,
                        }
                    },
                    {
                        "text": prompt,
                    }
                ]
            }
        ]
    });

    let aa = serde_json::to_string(&body).unwrap();
    // println!("{}", aa);

    let request = Request::builder()
        .method(Method::Post)
        .header("Content-Type", "application/json")
        .header("x-goog-api-key", api_key)
        .uri(api_uri)
        .body(aa)
        .build();

    // Send the request and await the response
    let response: Response = spin_sdk::http::send(request).await?;

    if *response.status() == 200u16 {
        //
        //
        let body = str::from_utf8(response.body()).unwrap();
        let a: Value = serde_json::from_str(body)?;
        let description = a.get("candidates").unwrap()[0]
            .get("content")
            .unwrap()
            .get("parts")
            .unwrap()[0]
            .get("text")
            .unwrap()
            .as_str()
            .unwrap()
            .trim()
            .to_string();
        return Ok(Some(description));
    } else {
        println!("Received from google gemini api: {}", response.status());
    }

    Ok(None)
}

pub fn get_multipart_boundary(req: &Request) -> anyhow::Result<String> {
    let boundary = req.header("content-type").unwrap();
    let a = boundary.as_str().unwrap();
    if &a[..30] == "multipart/form-data; boundary=" {
        let split = a.split("boundary=").collect::<Vec<&str>>();
        let a = split[1];
        return Ok(a.to_string());
    }
    bail!("Can't find boundary from header")
}

pub fn allowed_mime_type(mime: &Mime) -> anyhow::Result<Option<&str>> {
    let allowed_mime_and_extension = HashMap::from([
        (mime::IMAGE_GIF, "gif"),
        (mime::IMAGE_PNG, "png"),
        (mime::IMAGE_JPEG, "jpg"),
    ]);

    if let Some(ext) = allowed_mime_and_extension.get(&mime) {
        return Ok(Some(ext));
    }

    Ok(None)
}

fn encode_image_to_base64(image_bytes: Vec<u8>) -> Result<String> {
    Ok(general_purpose::STANDARD.encode(image_bytes))
}
