Amp Sensor Backend
==================

This is the backend for an amperimeter sensor. It is a simple REST API that
allows storing amperimeter readings from an electric circuit to perform
later analysis on them.

This project was built originally to both allow us at home to see quickly over
wifi if any powerful devices were simultaneously connected at home, and later to
also automatically use any remaining power budget to charge an EV we own, but
delaying the charge if no power budget is available.

We originally thought about creating an EVSE for that matter, but we found out
that for our EV platform, the car allows setting any amperage as low as 0A in 1A
increments, so we decided to implement this functionality as a Rocket fairing
here instead of hacking 1-phase dumb EVSE to support ISO 15118-2 XML messages.

For that matter we can rely on a cheapo $60 dumb 16A EVSE charger.

The amperimeter sensor can be built with a simple ACS712 sensor or a SCT013
sensor, and a ESP32 or similar microcontroller. The ESP32 should send the
amperimeter readings to this backend using the following format:

```
curl -X POST -H "Content-Type: application/json" -d '{"amps": 10.0, "watts": 2200.0}' http://localhost:8000/log/$TOKEN/
```

The backend will store the readings in a SQLite database and will allow querying
the readings to perform analysis on them.

It will also automatically query the Tessie API to check if the car is nearby
the charger and is charging. If it is, the backend will automatically increase or
decrease the amperage requested by the car to match the power budget available.

All these options can be set up in the [Rocket.toml](Rocket.example.toml) file.
