#!/bin/bash

cross build --target=aarch64-unknown-linux-musl --release

scp target/aarch64-unknown-linux-musl/release/amp-sensor-tessie-backend burato:usr/amp-sensor-tessie-backend/amp-sensor-tessie-backend
