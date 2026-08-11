# Ingest-to-pixel flow, with measured stage timings

Every number below comes from a committed artifact — see
[`ingest-to-pixel-flow.notes.md`](ingest-to-pixel-flow.notes.md) for the figure-by-figure
provenance (bench medians are criterion medians on Apple M2 Max; HTTP percentiles are the
committed load baseline; whole-pipeline numbers are the e2e harness). Stages labeled
*no committed timing* have never been measured in a committed artifact and carry no number
on purpose.

```mermaid
flowchart TD
    subgraph ingest["Ingest half — whole-pipeline ingest-to-pixel: 297 ms and 801 ms local, 535 ms CI, budget 10 000 ms"]
        EV["Granule event arrives<br/>arrival stamps the north-star clock<br/>(no committed timing)"] --> ORCH{"Ingest<br/>orchestrator"}
        ORCH -->|"clean COG / Zarr"| REG["Register asset<br/>(no committed timing)"]
        ORCH -->|"legacy NetCDF / HDF"| VREF["Virtual-reference manifest<br/>14 ms warm, 29 ms cold<br/>(prototype-grade measurement)"]
        VREF --> REG
        REG --> CAT["Catalog upsert, pgstac<br/>(no committed timing)"]
        CAT --> SRV["Layer servable"]
    end

    SRV -.-> REQ

    subgraph serve["Serve half — end-to-end HTTP p50: 22.27 ms hot cache, 653.88 ms cold live render"]
        REQ["GET tile z/y/x"] --> RES["Resolve layer + RenderSpec<br/>(no committed timing)"]
        RES --> PLAN["Planner<br/>45–73 ns"]
        PLAN -->|"cache_hit"| HIT["Serve stored tile<br/>storm p50 22.27 ms end-to-end"]
        PLAN -->|"overview / live"| WIN["Source window computation<br/>13.4 µs"]
        WIN --> READ["Read source byte ranges<br/>(no committed isolated timing;<br/>read_ms exists per-request in the Trace)"]
        READ --> WARP["Reproject + resample, per band<br/>8.0 ms nearest, 8.9 ms bilinear"]
        WARP --> EVAL["Render IR: band math, composite, colormap<br/>3.2 ms grayscale, 3.3 ms RdYlGn"]
        EVAL --> ENC["Encode PNG<br/>13.0 ms"]
        ENC --> WT["Write-through cache<br/>(no committed timing)"]
        WT --> OUT["Encoded tile + Trace"]
        HIT --> OUT
    end

    OUT -.- NOTE["Full composite bench, window→read→warp→eval→encode,<br/>NDVI z12 from an in-memory store: 37.3 ms median"]
```

Reading notes:

- The stage benches deliberately do **not** sum to the 37.3 ms composite: the composite reads
  two bands concurrently from an in-memory store and includes trace assembly, while the warp
  bench measures a single band.
- The hot-cache and cold-live figures are end-to-end HTTP percentiles under concurrency
  (oha storms), not single-request stage sums.
- The ingest-half total (297/801/535 ms) has no committed per-stage breakdown; only the total
  is measured, and up to ~500 ms of it is e2e poll granularity.
