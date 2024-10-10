# Testing

The testing facade depends on [mitmproxy](https://mitmproxy.org/) being installed on your device.


In order to re-generate the flows, in one process, run
```sh
 mitmdump -w mitmproxy/flows
 ```
 and in another process run
 ```sh
 cargo test
 ```
 This requires an internet connection to succeed.
