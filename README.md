# LWDProxy RS

`lwdproxy_rs` is a Zcash Lightwalletd proxy and mirror. It can be used to deploy
a Lightwalletd server for private usage or for development. It will download
blocks from an upstream server and store them locally in a LMDB database. You
can instruct it to trim blocks and start at a given height. By default, it will
fetch all the blocks since the Sapling activation.

## Installation

Copy the binary for your platform or build from source. We offer binaries for
Linux x64 and arm64, MacOS Apple Chips, and Windows.

Take the `App.json` too. It is the configuration file.

```json
{
    "db_path": "zec.mdb",
    "origin": ["https://zec.rocks"],
    "bind_address": "0.0.0.0",
    "port": 9067,
    "min_height": 2100000,
    "tls": false,
    "cert_path": "fullchain.pem",
    "key_path": "privkey.pem"
}
```

The `db_path` is a path to a *directory* that you must create empty.
`origin` is an array of URLs to other LightWalletd servers. `lwdproxy_rs` will
load balance between them. `bind_address` is the IP address where the server
listens. It should be one of your network address or `0.0.0.0` to bind to every
interface. `port` is the binding port. `min_height` is the lowest height the
server downloads blocks. `tls` activates TLS. Then you need to provide the
public CERT in `cert_path` (it should be a full chain PEM). And finally,
`key_path` gives the path to the private key.

::: tip
You can save a lot of time and disk space by using a min_height > 0.
But be sure that your wallets are created *after* this height.
:::

A complete installation takes ~20 GB of disk space at this time (Block Height 3 M).
