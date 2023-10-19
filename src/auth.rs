use reqwest::Client;

pub async fn authenticate_token(auth_uri: &str, token: &str) -> Result<(), ()> {
    println!("authenticating sleep ing");
    let client = Client::new();
    let token = if token.starts_with("Token ") {
        token.trim_start_matches("Token ")
    } else {
        token
    };

    let auth_address = format!("https://{}/api/exchanges", auth_uri);
    println!("auth address: {}", auth_address);

    let res = client
        .get(auth_address)
        .header("Authorization", format!("Token {}", token))
        .send()
        .await;

    match res {
        Ok(res) => match res.status() {
            reqwest::StatusCode::OK => {
                println!("auth success");
                Ok(())
            }
            _ => {
                println!("auth failed");
                Err(())
            }
        },
        Err(e) => {
            println!("error: {:?}", e);
            Err(())
        }
    }
}
