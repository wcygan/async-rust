# Async `tail -f`

## Description

`tail -f` is a command that outputs the last part of a file and then waits for more data to be appended to the file. The `-f` option is used to follow a file in real-time.

This program emulates this functionality, continuously waiting for file changes and printing the new content.

## Usage

In one process, I've started the [file writer](../5-file-writer/readme.md):

```bash
cargo run -- --filepath log.txt
FnSQbqdXjGMbx1ZFz5cKS
kkOH1JkegICOgqc-bMb-9
TJJAjDxe_4LsbZpNRfRVZ
P5reB69LF0OQo4394xPnd
KzwmLulc4UiCD1KIcg1fY
QbPAiUlgHKwtLRSu1oO6z
YcqT5cSCbIYGfdg4gTOGG
ZfkY8zJfXgAeaR_L3pIKS
PnFgYKwmM9bb7f-F6IKF8
KSi5AmDlbq-ItIJW7SvPR
lR_N_vB8N7kFwEF6XKhwD
wVHCJSSh_140AihckYDze
7krmVsI1Swzn9WQShiec6
```

In another process, I've started [async-tail-f](../6-async-tail-f/readme.md):

```bash
cargo run -- --filepath ../5-file-writer/log.txt
FnSQbqdXjGMbx1ZFz5cKS
kkOH1JkegICOgqc-bMb-9
TJJAjDxe_4LsbZpNRfRVZ
P5reB69LF0OQo4394xPnd
KzwmLulc4UiCD1KIcg1fY
QbPAiUlgHKwtLRSu1oO6z
YcqT5cSCbIYGfdg4gTOGG
ZfkY8zJfXgAeaR_L3pIKS
PnFgYKwmM9bb7f-F6IKF8
KSi5AmDlbq-ItIJW7SvPR
lR_N_vB8N7kFwEF6XKhwD
wVHCJSSh_140AihckYDze
7krmVsI1Swzn9WQShiec6
```