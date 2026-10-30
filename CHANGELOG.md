# Changelog


## 1.1.4 — 3 November 2026

A maintenance release.

### Fixes

- Relay list refreshes without freezing the window.

## 1.1.3 · 2026-11-01

- Route race no longer stalls on servers that answer slowly.
- Relay list refreshes without dropping the active route.
- Ping graph draws correctly on high-DPI screens.
- Quitter message when no route can help, in plainer words.

## 1.1.1 · 2026-10-09

- The verdict names the reason when no route can help, instead of selling one.
- Only the game's own UDP traffic takes the new path; everything else stays.

## 1.1.2 · 2026-10-30

- **Lower my ping.** One button. Detour races your normal path against faster ones every 5 seconds and keeps the winner.
- **No more lag spikes.** Each packet travels two ways at once, so a lost one no longer becomes a spike.
- **An honest answer.** One sentence on why you lag. Wi-Fi, a busy router and downloads are checked first, and games that already have a fast path are left alone.
- **Your own free server.** Set one up on Oracle Cloud's free tier in about 3 minutes, or use the free community paths.
- **Show details** has the path race, the lag-spike graph and the technical names.
- Only the game's traffic takes the new path. Games are picked up from `games.toml`.
- One portable file for Windows 10 and 11 (x64).
