# Apply lag on main: complete tables

Generated from the runs of `report-full.md`. The first two tables come from the first pass of the
grid, which ran without `--measure-apply`, except the `arrival no-op` rows. The apply backlog table
and the `arrival no-op` rows come from the second pass, and the last table from 30 more pairs of
runs of the saturated 2 ms row.

Runs per row: 5, in shuffled order per round. Clients: 256. Load average (1 min) at run start: min 6.3, median 14.8, max 26.0. Failed writes: 0.

## Throughput and client latency

| state machine | load | n | writes/s | min–max writes/s | client p50 µs | client p99 µs |
| :-- | --: | --: | --: | --: | --: | --: |
| lan, apply 100 µs/call | 10,000/s | 20,000 | 9,992 | 9,985–9,995 | 1,737 | 3,113 |
| lan, apply 100 µs/call | 50,000/s | 100,000 | 49,951 | 49,932–49,959 | 1,753 | 3,178 |
| lan, apply 100 µs/call | saturated | 493,824 | 246,646 | 240,667–249,955 | 1,032 | 1,229 |
| lan, apply 500 µs/call | 10,000/s | 20,000 | 9,987 | 9,985–9,992 | 2,523 | 3,637 |
| lan, apply 500 µs/call | 50,000/s | 100,000 | 49,949 | 49,927–49,955 | 2,556 | 3,604 |
| lan, apply 500 µs/call | saturated | 342,016 | 169,419 | 154,965–170,415 | 1,524 | 1,704 |
| lan, apply 2 ms/call | 10,000/s | 20,000 | 9,972 | 9,971–9,976 | 5,046 | 6,750 |
| lan, apply 2 ms/call | 50,000/s | 100,000 | 49,840 | 44,000–49,888 | 5,571 | 12,321 |
| lan, apply 2 ms/call | saturated | 102,912 | 68,320 | 52,584–75,622 | 3,506 | 5,177 |
| lan, apply 100 µs/call + 10 µs/entry | 10,000/s | 20,000 | 9,991 | 9,990–9,995 | 1,917 | 3,015 |
| lan, apply 100 µs/call + 10 µs/entry | 50,000/s | 100,000 | 49,940 | 49,939–49,956 | 2,654 | 4,162 |
| lan, apply 100 µs/call + 10 µs/entry | saturated | 128,768 | 65,239 | 40,892–68,056 | 4,063 | 4,588 |
| cloud, apply 500 µs/call | 10,000/s | 20,000 | 9,978 | 9,977–9,979 | 4,522 | 5,571 |
| cloud, apply 500 µs/call | saturated | 135,168 | 69,026 | 47,959–69,296 | 3,736 | 4,030 |
| arrival no-op | 10,000/s | 20,000 | 9,994 | 9,975–10,000 | 1,245 | 2,327 |
| arrival no-op | 50,000/s | 100,000 | 49,966 | 49,948–49,981 | 1,262 | 2,359 |

## Leader stage latencies

Stage latencies are per entry, on the leader, in µs. The stages 1–4 column is the sum of the four stage p50s up to commit, not a percentile of the sum.

| state machine | load | stages 1–4 p50 sum | persisted→committed p50 / p99 | committed→applied p50 / p99 | proposed→applied p50 / p99 | after commit, share of p50 | entries per apply command, mean | stage samples |
| :-- | --: | --: | --: | --: | --: | --: | --: | --: |
| lan, apply 100 µs/call | 10,000/s | 664 | 351 / 426 | 239 / 299 | 923 / 1,022 | 26% | 13 | 11,549 |
| lan, apply 100 µs/call | 50,000/s | 693 | 365 / 489 | 239 / 305 | 955 / 1,160 | 25% | 59 | 32,086 |
| lan, apply 100 µs/call | saturated | 746 | 400 / 567 | 372 / 499 | 1,074 / 1,395 | 35% | 254 | 84,532 |
| lan, apply 500 µs/call | 10,000/s | 655 | 349 / 492 | 712 / 1,467 | 1,417 / 2,838 | 50% | 12 | 11,617 |
| lan, apply 500 µs/call | 50,000/s | 679 | 358 / 526 | 722 / 1,292 | 1,417 / 2,014 | 51% | 63 | 37,704 |
| lan, apply 500 µs/call | saturated | 731 | 386 / 570 | 768 / 1,010 | 1,517 / 2,375 | 51% | 133 | 55,533 |
| lan, apply 2 ms/call | 10,000/s | 655 | 350 / 428 | 3,176 / 4,837 | 3,798 / 5,177 | 84% | 12 | 10,809 |
| lan, apply 2 ms/call | 50,000/s | 703 | 369 / 531 | 3,700 / 5,121 | 4,313 / 5,872 | 86% | 62 | 35,120 |
| lan, apply 2 ms/call | saturated | 749 | 402 / 589 | 2,866 / 5,058 | 3,615 / 5,674 | 79% | 215 | 75,362 |
| lan, apply 100 µs/call + 10 µs/entry | 10,000/s | 642 | 342 / 453 | 352 / 505 | 983 / 1,260 | 36% | 10 | 9,530 |
| lan, apply 100 µs/call + 10 µs/entry | 50,000/s | 689 | 368 / 481 | 872 / 1,527 | 1,682 / 2,517 | 52% | 55 | 31,623 |
| lan, apply 100 µs/call + 10 µs/entry | saturated | 764 | 400 / 577 | 3,229 / 3,912 | 3,843 / 4,823 | 84% | 179 | 59,924 |
| cloud, apply 500 µs/call | 10,000/s | 2,711 | 1,351 / 1,677 | 810 / 1,320 | 3,379 / 3,965 | 24% | 12 | 10,044 |
| cloud, apply 500 µs/call | saturated | 2,893 | 1,427 / 1,742 | 960 / 1,271 | 3,835 / 4,562 | 25% | 256 | 114,688 |

## Apply backlog (second pass, `--measure-apply`)

Runs per row: 5, in shuffled order per round. Load average (1 min) at run start: min 7.3, median 12.0, max 32.0. Committed minus applied index, in entries, sampled at every metrics report of the node; percentiles are bucketed, maxima are exact. Medians over runs.

| state machine | load | leader p50 / p99 / max | leader max, range | leader samples | any node max, range | entries per apply() p50 / p90 | apply() calls |
| :-- | --: | --: | --: | --: | --: | --: | --: |
| lan, apply 100 µs/call | 10,000/s | 0 / 20 / 140 | 20–250 | 11,652 | 20–270 | 10 / 10 | 1,881 |
| lan, apply 100 µs/call | 50,000/s | 0 / 100 / 100 | 100–256 | 10,488 | 100–256 | 50 / 50 | 1,830 |
| lan, apply 100 µs/call | saturated | 0 / 256 / 256 | 256–256 | 10,800 | 256–512 | 256 / 256 | 1,822 |
| lan, apply 500 µs/call | 10,000/s | 10 / 20 / 30 | 24–256 | 10,173 | 24–256 | 10 / 20 | 1,598 |
| lan, apply 500 µs/call | 50,000/s | 50 / 100 / 150 | 125–256 | 10,858 | 125–256 | 50 / 100 | 1,613 |
| lan, apply 500 µs/call | saturated | 133 / 256 / 256 | 256–256 | 13,674 | 256–256 | 113 / 205 | 2,479 |
| lan, apply 2 ms/call | 10,000/s | 40 / 70 / 80 | 80–80 | 9,308 | 80–80 | 20 / 40 | 799 |
| lan, apply 2 ms/call | 50,000/s | 256 / 256 / 256 | 256–296 | 5,764 | 300–512 | 106 / 155 | 783 |
| lan, apply 2 ms/call | saturated | 256 / 256 / 256 | 256–256 | 5,874 | 256–512 | 245 / 253 | 832 |
| lan, apply 100 µs/call + 10 µs/entry | 10,000/s | 10 / 20 / 20 | 20–256 | 11,454 | 20–256 | 10 / 20 | 1,707 |
| lan, apply 100 µs/call + 10 µs/entry | 50,000/s | 50 / 151 / 256 | 200–256 | 11,618 | 200–512 | 50 / 100 | 1,655 |
| lan, apply 100 µs/call + 10 µs/entry | saturated | 256 / 256 / 256 | 256–256 | 5,150 | 489–512 | 163 / 256 | 689 |
| cloud, apply 500 µs/call | 10,000/s | 10 / 30 / 40 | 30–226 | 7,083 | 30–226 | 10 / 20 | 1,619 |
| cloud, apply 500 µs/call | saturated | 0 / 256 / 256 | 256–256 | 3,465 | 256–256 | 256 / 256 | 465 |

## Saturated 2 ms-per-call row, with and without `--measure-apply`

30 pairs at `ae7a4509`, each pair one run without and one with the flag, in random order within the pair. Load average (1 min) after each run: min 5.4, median 7.9, max 15.3. A run is slow when its committed→applied p50 is above 3.8 ms.

| configuration | runs | writes/s median | writes/s range | committed→applied p50 median µs | slow runs |
| :-- | --: | --: | --: | --: | --: |
| without `--measure-apply` | 30 | 55,981 | 50,832–75,706 | 4,422 | 21 |
| with `--measure-apply` | 30 | 55,034 | 50,900–75,741 | 4,457 | 21 |
| fast runs, both configurations | 18 | 73,204 | 56,342–75,741 | 2,801 | – |
| slow runs, both configurations | 42 | 51,338 | 50,832–63,294 | 4,462 | – |

Mann-Whitney U on writes/s: 476 of 900; two-sided permutation p = 0.71 (20,000 permutations).
