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
- `air-quality-history.html` manages persistent storage, clock synchronization,
  history retrieval, graphing, and CSV export.
