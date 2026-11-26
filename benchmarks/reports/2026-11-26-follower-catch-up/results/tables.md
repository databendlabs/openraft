Runs per row: 5, in shuffled order per round. Clients: 256. Node 2 is cut off 1000 ms after the clients start. Load average (1 min) at run start: min 4.2, median 6.0, max 10.7. Failed writes: 0. Runs that did not catch up: 0.

## Follower catch-up

Times in ms after the reconnection, medians over runs.

| state machine | load | outage | entries behind | first new entry | log reaches target | applied target | apply after log | follower max committed unapplied | apply() calls | entries per call p50 / max |
| :-- | --: | :-- | --: | --: | --: | --: | --: | --: | --: | --: |
| lan, apply 100 µs/call | 10,000/s | 1 s | 10,010 | 201.1 | 604.7 | 604.7 | 0.0 | 4,846 | 11 | 1,024 / 1,024 |
| lan, apply 100 µs/call | saturated | 1 s | 248,624 | 202.4 | 639.9 | 648.8 | 8.7 | 64,512 | 261 | 1,024 / 1,024 |
| lan, apply 500 µs/call | 10,000/s | 1 s | 10,010 | 202.2 | 605.4 | 609.6 | 4.2 | 12,988 | 10 | 1,024 / 1,024 |
| lan, apply 500 µs/call | saturated | 1 s | 142,279 | 201.2 | 626.9 | 690.3 | 65.9 | 180,388 | 140 | 1,024 / 1,024 |
| lan, apply 2 ms/call | 10,000/s | 1 s | 10,010 | 202.5 | 605.4 | 625.2 | 19.5 | 15,046 | 10 | 1,024 / 1,024 |
| lan, apply 2 ms/call | saturated | 1 s | 52,463 | 201.7 | 614.0 | 733.8 | 119.8 | 78,974 | 52 | 1,024 / 1,024 |
| lan, 100 µs/call + 10 µs/entry | 10,000/s | 1 s | 10,010 | 201.1 | 605.8 | 708.6 | 101.9 | 15,136 | 10 | 1,024 / 1,024 |
| lan, 100 µs/call + 10 µs/entry | saturated | 1 s | 72,810 | 201.5 | 618.2 | 1,409.1 | 791.0 | 116,842 | 72 | 1,024 / 1,024 |
| cloud, apply 500 µs/call | 10,000/s | 1 s | 10,010 | 201.6 | 607.9 | 611.4 | 3.6 | 11,994 | 11 | 1,024 / 1,024 |
| cloud, apply 500 µs/call | saturated | 1 s | 67,962 | 202.6 | 619.8 | 649.5 | 25.5 | 76,768 | 68 | 1,024 / 1,024 |
| lan, apply 2 ms/call | 10,000/s | 5 s | 50,010 | 201.0 | 611.2 | 717.1 | 107.1 | 53,058 | 49 | 1,024 / 1,024 |
| lan, 100 µs/call + 10 µs/entry | 10,000/s | 5 s | 50,010 | 202.3 | 610.4 | 1,152.5 | 542.1 | 55,146 | 49 | 1,024 / 1,024 |
| lan, apply 100 µs/call | 10,000/s | 1 s, backoff 10 ms | 10,010 | 11.5 | 34.0 | 34.7 | 0.6 | 4,206 | 10 | 1,024 / 1,024 |
| lan, apply 100 µs/call | saturated | 1 s, backoff 10 ms | 236,560 | 11.6 | 67.8 | 73.1 | 5.1 | 45,629 | 232 | 1,024 / 1,024 |

## Leader writes per phase

writes/s and client p99 in ms, medians over runs. A phase with no writes in some runs is left out of those runs' medians; `-` means no run had writes in it.

| state machine | load | outage | before | outage | catch-up | after |
| :-- | --: | :-- | --: | --: | --: | --: |
| lan, apply 100 µs/call | 10,000/s | 1 s | 9,979, 3.1 | 9,998, 3.1 | 10,021, 3.1 | 10,000, 3.1 |
| lan, apply 100 µs/call | saturated | 1 s | 242,823, 1.2 | 248,409, 1.2 | 249,014, 1.2 | 245,379, 1.2 |
| lan, apply 500 µs/call | 10,000/s | 1 s | 9,969, 3.5 | 10,001, 3.5 | 10,014, 3.4 | 9,998, 3.5 |
| lan, apply 500 µs/call | saturated | 1 s | 166,600, 1.7 | 141,934, 2.1 | 153,744, 2.0 | 143,668, 2.0 |
| lan, apply 2 ms/call | 10,000/s | 1 s | 9,950, 6.9 | 9,991, 7.1 | 10,013, 6.9 | 9,996, 6.9 |
| lan, apply 2 ms/call | saturated | 1 s | 74,384, 3.7 | 52,394, 5.1 | 51,743, 5.2 | 51,553, 5.2 |
| lan, 100 µs/call + 10 µs/entry | 10,000/s | 1 s | 9,978, 3.0 | 10,000, 3.1 | 10,018, 3.1 | 9,999, 3.0 |
| lan, 100 µs/call + 10 µs/entry | saturated | 1 s | 62,380, 4.5 | 72,884, 4.4 | 73,179, 3.6 | 73,097, 3.6 |
| cloud, apply 500 µs/call | 10,000/s | 1 s | 9,955, 5.6 | 9,993, 5.6 | 10,017, 5.6 | 9,998, 5.6 |
| cloud, apply 500 µs/call | saturated | 1 s | 67,865, 5.4 | 68,067, 4.0 | 69,312, 5.8 | 69,026, 4.1 |
| lan, apply 2 ms/call | 10,000/s | 5 s | 9,945, 6.8 | 10,001, 7.0 | 9,984, 7.0 | 10,002, 6.7 |
| lan, 100 µs/call + 10 µs/entry | 10,000/s | 5 s | 9,977, 2.9 | 10,000, 3.1 | 10,006, 3.1 | 10,000, 3.0 |
| lan, apply 100 µs/call | 10,000/s | 1 s, backoff 10 ms | 9,978, 3.1 | 10,002, 3.1 | 10,030, 3.1 | 10,000, 3.1 |
| lan, apply 100 µs/call | saturated | 1 s, backoff 10 ms | 239,127, 1.2 | 236,442, 1.3 | 243,863, 1.2 | 235,521, 1.2 |

## Saturated rows, run by run

writes/s in thousands, then client p50 and p99 in ms, of the writes that completed in each phase. Load is the 1-minute load average at the start of the run.

| state machine | outage | round | load | before | outage | catch-up | after |
| :-- | :-- | --: | --: | --: | --: | --: | --: |
| cloud, apply 500 µs/call | 1 s | 1 | 6.7 | 47.4k, 4.03, 23.6 | 42.0k, 4.52, 47.7 | 69.6k, 3.64, 6.5 | 71.4k, 3.57, 4.0 |
| cloud, apply 500 µs/call | 1 s | 2 | 8.3 | 60.2k, 3.80, 13.1 | 68.1k, 3.77, 4.0 | 67.0k, 3.67, 8.1 | 70.6k, 3.60, 4.8 |
| cloud, apply 500 µs/call | 1 s | 3 | 7.3 | 67.9k, 3.74, 5.4 | 66.4k, 3.80, 5.0 | 68.5k, 3.67, 5.8 | 60.3k, 3.64, 14.2 |
| cloud, apply 500 µs/call | 1 s | 4 | 4.5 | 69.1k, 3.74, 4.0 | 68.8k, 3.77, 3.9 | 69.3k, 3.74, 3.9 | 69.0k, 3.74, 3.9 |
| cloud, apply 500 µs/call | 1 s | 5 | 5.3 | 69.1k, 3.74, 3.9 | 68.8k, 3.74, 3.9 | 69.6k, 3.70, 3.9 | 67.7k, 3.77, 4.1 |
| lan, apply 500 µs/call | 1 s | 1 | 6.9 | 167.5k, 1.54, 1.7 | 143.4k, 1.90, 2.0 | 132.4k, 1.95, 2.0 | 141.7k, 1.90, 2.0 |
| lan, apply 500 µs/call | 1 s | 2 | 9.8 | 136.4k, 1.85, 3.0 | 141.9k, 1.90, 2.6 | 153.7k, 1.56, 2.2 | 116.8k, 1.61, 11.3 |
| lan, apply 500 µs/call | 1 s | 3 | 4.8 | 166.6k, 1.54, 1.7 | 132.7k, 1.95, 2.2 | 132.8k, 1.95, 2.0 | 148.7k, 1.64, 2.0 |
| lan, apply 500 µs/call | 1 s | 4 | 4.5 | 163.1k, 1.57, 1.7 | 132.4k, 1.95, 2.0 | 158.2k, 1.56, 2.0 | 160.8k, 1.56, 2.0 |
| lan, apply 500 µs/call | 1 s | 5 | 6.1 | 167.0k, 1.54, 1.7 | 161.4k, 1.57, 2.1 | 161.4k, 1.56, 3.0 | 143.7k, 1.88, 2.0 |
| lan, apply 100 µs/call | 1 s | 1 | 8.3 | 125.4k, 1.18, 13.6 | 230.0k, 1.08, 1.9 | 239.3k, 1.06, 1.2 | 238.3k, 1.06, 1.4 |
| lan, apply 100 µs/call | 1 s | 2 | 9.1 | 248.9k, 1.02, 1.2 | 248.4k, 1.02, 1.2 | 249.0k, 1.02, 1.2 | 245.4k, 1.04, 1.2 |
| lan, apply 100 µs/call | 1 s | 3 | 5.4 | 247.5k, 1.03, 1.1 | 250.5k, 1.02, 1.2 | 254.8k, 1.01, 1.1 | 250.4k, 1.02, 1.1 |
| lan, apply 100 µs/call | 1 s | 4 | 5.1 | 242.8k, 1.05, 1.2 | 250.6k, 1.02, 1.2 | 251.8k, 1.02, 1.1 | 248.7k, 1.03, 1.1 |
| lan, apply 100 µs/call | 1 s | 5 | 5.4 | 237.5k, 1.08, 1.2 | 235.7k, 1.08, 1.8 | 239.9k, 1.06, 1.2 | 235.5k, 1.10, 1.2 |
| lan, apply 100 µs/call | 1 s, backoff 10 ms | 1 | 8.7 | 239.1k, 1.08, 1.2 | 232.4k, 1.10, 1.5 | 168.5k, 1.26, 9.0 | 231.8k, 1.08, 1.7 |
| lan, apply 100 µs/call | 1 s, backoff 10 ms | 2 | 7.7 | 242.4k, 1.02, 1.3 | 234.9k, 1.06, 1.8 | 248.7k, 1.03, 1.2 | 244.2k, 1.03, 1.3 |
| lan, apply 100 µs/call | 1 s, backoff 10 ms | 3 | 6.3 | 250.7k, 1.02, 1.1 | 250.7k, 1.02, 1.2 | 249.4k, 1.02, 1.2 | 248.4k, 1.02, 1.2 |
| lan, apply 100 µs/call | 1 s, backoff 10 ms | 4 | 4.6 | 237.9k, 1.08, 1.2 | 237.8k, 1.08, 1.2 | 243.9k, 1.06, 1.2 | 235.5k, 1.08, 1.2 |
| lan, apply 100 µs/call | 1 s, backoff 10 ms | 5 | 5.5 | 237.0k, 1.08, 1.2 | 236.4k, 1.08, 1.3 | 228.5k, 1.13, 1.2 | 233.5k, 1.10, 1.2 |
| lan, 100 µs/call + 10 µs/entry | 1 s | 1 | 8.6 | 74.7k, 3.44, 3.9 | 73.6k, 3.51, 4.8 | 73.2k, 3.51, 3.6 | 73.1k, 3.54, 3.6 |
| lan, 100 µs/call + 10 µs/entry | 1 s | 2 | 9.5 | 60.1k, 4.33, 4.6 | 72.9k, 3.54, 4.3 | 72.8k, 3.54, 3.6 | 72.5k, 3.57, 3.6 |
| lan, 100 µs/call + 10 µs/entry | 1 s | 3 | 5.3 | 62.4k, 4.16, 4.5 | 65.4k, 3.93, 4.4 | 76.0k, 3.38, 3.6 | 75.6k, 3.41, 3.6 |
| lan, 100 µs/call + 10 µs/entry | 1 s | 4 | 4.2 | 62.4k, 4.13, 4.5 | 68.7k, 3.70, 4.4 | 62.5k, 4.16, 4.5 | 69.8k, 3.54, 4.8 |
| lan, 100 µs/call + 10 µs/entry | 1 s | 5 | 5.5 | 62.4k, 4.16, 4.5 | 73.2k, 3.51, 4.4 | 76.1k, 3.41, 3.6 | 76.4k, 3.38, 3.6 |
| lan, apply 2 ms/call | 1 s | 1 | 7.7 | 51.4k, 5.05, 5.9 | 51.7k, 5.05, 5.2 | 51.7k, 5.11, 5.2 | 51.6k, 5.11, 5.2 |
| lan, apply 2 ms/call | 1 s | 2 | 8.6 | 53.6k, 5.11, 5.2 | 51.6k, 5.11, 5.2 | 51.2k, 5.11, 5.2 | 51.3k, 5.11, 5.2 |
| lan, apply 2 ms/call | 1 s | 3 | 5.2 | 74.4k, 3.44, 3.7 | 52.4k, 5.05, 5.1 | 51.3k, 5.11, 5.2 | 50.9k, 5.11, 5.2 |
| lan, apply 2 ms/call | 1 s | 4 | 5.4 | 76.2k, 3.41, 3.6 | 63.9k, 3.51, 5.1 | 52.5k, 4.92, 5.2 | 52.5k, 4.98, 5.2 |
| lan, apply 2 ms/call | 1 s | 5 | 6.0 | 75.7k, 3.41, 3.6 | 69.8k, 3.44, 5.1 | 52.3k, 4.98, 6.0 | 52.5k, 4.98, 5.2 |
