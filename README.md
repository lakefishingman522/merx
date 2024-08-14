# MERX - CoinRoutes Market Data
   
Merx is a multi-threaded market data handler that pulls and serves data from cbag to clients. It is built purely in Rust and uses rust's tokio runtime and axum, a web application framework.

Merx will de-duplicate subscriptions. I.e. if multiple clients request the same subscription, only a single subscription will be made to cbag.

Detailed documentation for merx can be found [here](https://dhuita5s1xx86.cloudfront.net/apis/merx/merx.html)
   
            
## Building and running for prod

To build merx optimally for production run:
```   
cargo build --release
```
    
This will create an executable named `merx` in the `target/releases` directory which can be run:
``` 
./target/release/merx --cbag-uri internal-prod-cbag-726087086.ap-northeast-1.elb.amazonaws.com:8080 --auth-uri portal.coinroutes.com --prod --token <TOKEN>
```

## Building for local development
You can build and run merx using a single command for development. This does not build an optimal version of merx so should not be used for benchmarking but build times are quicker than building for release so this is ideal when developing locally.

```
cargo run --bin merx -- --cbag-uri internal-prod-cbag-1289855758.us-east-1.elb.amazonaws.com:8080 --auth-uri portal.coinroutes.com --token <TOKEN>
```


## Command Line Arguments

- **`--cbag-uri`**: *string* (required) - the uri of the cbag or load balancer to request market data from i.e. `internal-prod-cbag-1289855758.us-east-1.elb.amazonaws.com:8080` for the US load balancer
- **`--cbag-depth-uri`**: *string* (optional) - If depth is to be requested from a different uri, specify here  (will default to using cbag-uri for depth)
- **`--auth-uri`**: *string* (required) - Uri of the auth server. normally `portal.coinroutes.com`
- **`--token`**: *string* (required) - A valid token for the auth server
- **`--port`**: *string* (optional) - Set a specific port for Merx to run on (Default: 5050)
- **`--prod`**: *flag* (optional) - Use this parameter if you would like merx to run on `0.0.0.0` for prod. Else it will run on `127.0.0.1`


## Other Details

Merx has a background task that will pull currency pairs from portal. These currency pairs are used to validate currency pairs, size filters and other parameters for incoming requests allowing merx to reject requests with pertinent error messages without having to subscribe to cbag.

When a user connects and requests data from an endpoint, merx will validate the user token from portal. This validation is cached and will be valid for a set duration therefore reducing the number of requests that need to made to portal.

If cbag is down or not accessible, or portal is down, merx will be unable to serve requests as expected.
