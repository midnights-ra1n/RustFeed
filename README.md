# RustFeed

RustFeed polls one or more RSS feeds on an interval and forwards new articles to a Discord channel as embeds via a webhook. Already-sent articles are remembered on disk so restarts don't resend everything, and that history is fully reset every 30 days to keep the state file small.

## Features

- Polls any number of RSS feeds on a configurable interval
- Posts new items to Discord as rich embeds via a webhook URL
- Persists which items have already been sent (`data/seen.json`) so restarts don't cause duplicate posts
- Automatically wipes that history every 30 days without dumping the whole feed backlog into Discord afterward
- Fails fast and loudly on invalid configuration instead of running silently broken

## Configuration

RustFeed reads a `config.toml` file from its working directory:

```toml
# Interval is in seconds
interval = 1800

webhook = "https://discord.com/api/webhooks/<id>/<token>"

feeds = [
  "https://www.clubic.com/feed/rss"
]
```

| Field      | Description                                              |
|------------|------------------------------------------------------------|
| `interval` | Delay between two poll cycles, in seconds                  |
| `webhook`  | Discord webhook URL to post embeds to                      |
| `feeds`    | List of RSS feed URLs to poll                               |

### Automatic config creation

If `config.toml` doesn't exist yet, RustFeed creates a default one for you and exits immediately with an explanatory error. This matters when running under Docker without a terminal (`docker run` in detached mode, Compose, an orchestrator): you don't need to `exec` into the container to bootstrap the file — just mount a *missing* `config.toml` path on the host, start the container once, edit the file it created, and restart.

> This only works if the file is genuinely missing on the host. An empty file (e.g. from `touch config.toml`) already "exists", so RustFeed won't overwrite it and will instead fail to parse it, crash-looping forever. Fetch the pre-filled template instead (see below), or make sure the file is fully absent before the first start.
>
> Fetch it with `curl` (used below) or, equivalently: `wget -O config.toml https://raw.githubusercontent.com/midnights-ra1n/RustFeed/main/config.toml`

RustFeed also validates the config on every startup and refuses to run if:
- `webhook` isn't a real Discord webhook URL (still the placeholder, empty, or malformed)
- `feeds` is empty

In any of these cases the process exits with a non-zero code and a clear message on stderr, then stops (or restarts and immediately stops again, if you're using a restart policy).

> **A first crash right after the initial setup is expected.** The very first start either has no `config.toml` (one gets generated for you) or one with placeholder values. RustFeed will exit on purpose so you can fill in your real Discord webhook URL and RSS feeds. Edit `config.toml`, then start (or restart) the container/binary again.

## Logs

RustFeed logs to stdout/stderr, so `docker logs`, `docker compose logs`, or your process supervisor will show:
- fetch errors per feed (network issues, invalid RSS, etc.) without stopping the other feeds
- send errors per Discord embed (e.g. an invalid webhook that only becomes invalid later, rate limits, etc.)
- configuration errors that caused the process to exit
- the 30-day history reset event, when it happens

## Data persistence

RustFeed keeps its "already sent" history in `data/seen.json` (created automatically). Mount `./data` as a volume so this survives container restarts/recreations — otherwise every restart will re-send the current feed contents once.

## Installation

### Option 1 — Docker image from GHCR

> The image is built and published automatically by GitHub Actions on every push to `main`, tagged `latest` and with the commit's short SHA. Note that a package pushed with `GITHUB_TOKEN` is **private by default** — after the first successful run, set its visibility to public from the package settings on GitHub if you want to pull it without authenticating.

```bash
mkdir rustfeed && cd rustfeed
curl -fsSL -o config.toml https://raw.githubusercontent.com/midnights-ra1n/RustFeed/main/config.toml

docker run -d \
  --name rustfeed \
  --restart unless-stopped \
  -v "$(pwd)/config.toml:/app/config.toml" \
  -v "$(pwd)/data:/app/data" \
  ghcr.io/midnights-ra1n/rustfeed:latest
```

Check the logs of the first run, edit the generated `config.toml` with your real webhook and feeds, then restart the container:

```bash
docker logs rustfeed
docker restart rustfeed
```

### Option 2 — Docker Compose

A `docker-compose.yml` is provided:

```yaml
services:
  rustfeed:
    build: .
    container_name: rustfeed
    restart: unless-stopped
    volumes:
      - ./config.toml:/app/config.toml
      - ./data:/app/data
```

```bash
curl -fsSL -o config.toml https://raw.githubusercontent.com/midnights-ra1n/RustFeed/main/config.toml
docker compose up -d --build

docker compose logs -f rustfeed   # confirm it's running, then edit config.toml with your webhook + feeds
docker compose restart rustfeed
```

To use the published GHCR image instead of building locally, replace `build: .` with:

```yaml
    image: ghcr.io/midnights-ra1n/rustfeed:latest
```

### Option 3 — Build the Docker image from source

```bash
git clone https://github.com/midnights-ra1n/RustFeed.git
cd RustFeed
docker build -t rustfeed .

curl -fsSL -o config.toml https://raw.githubusercontent.com/midnights-ra1n/RustFeed/main/config.toml
docker run -d \
  --name rustfeed \
  --restart unless-stopped \
  -v "$(pwd)/config.toml:/app/config.toml" \
  -v "$(pwd)/data:/app/data" \
  rustfeed
```

### Option 4 — Run with Rust directly (no Docker)

Requires a recent stable [Rust toolchain](https://rustup.rs/).

```bash
git clone https://github.com/midnights-ra1n/RustFeed.git
cd RustFeed
cargo build --release
./target/release/RustFeed
```

The first run will create `config.toml` in the current directory (if missing) and exit — edit it, then run the binary again. `data/seen.json` will be created next to it automatically.

## Contributing

See [CONTRIBUTION.md](CONTRIBUTION.md).
