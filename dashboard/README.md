# AirQuality dashboards

These are standalone HTML files with inline CSS and JavaScript. They have no
build step, package manager, CDN, or runtime dependency.

WebBluetooth requires a browser-approved context such as `localhost`. From this
directory, run:

```text
python3 -m http.server 8000
```

Then open `http://localhost:8000/air-quality.html` or
`http://localhost:8000/air-quality-history.html` in a compatible browser.

- `air-quality.html` shows live CO₂, NOx, temperature, pressure, sensor status,
  and RGB LED controls.
- `air-quality-history.html` manages persistent storage, DS3231/local clock
  synchronization and comparison, history retrieval, graphing, and CSV export.

The history dashboard reads the RTC service
(`7c3e4000-4e0a-4a69-9c57-6b5f3f1c0000`) and compares the board's locally
extrapolated Unix time with the computer clock using the BLE read midpoint and
round-trip latency.
