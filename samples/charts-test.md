# Chart rendering test

A paragraph before the charts to check vertical flow and spacing around the
rasterized chart blocks.

## Bar — single series

```chart
type: bar
title: Weekly active doctors
xlabel: Day
ylabel: Count
x: [Mon, Tue, Wed, Thu, Fri]
y: [12, 19, 14, 22, 30]
```

## Line — multi series

```chart
type: line
title: PraxisBench score over time
xlabel: Week
ylabel: Score
x: [1, 2, 3, 4, 5]
series:
  - name: Praxis
    y: [41, 47, 52, 58, 63]
  - name: Baseline
    y: [40, 41, 42, 41, 43]
```

## Pie

```chart
type: pie
title: Where the week went
data:
  Pipeline: 40
  Evals: 25
  Recruiting: 20
  Meetings: 15
```

## Scatter

```chart
type: scatter
title: Dose vs response
xlabel: Dose
ylabel: Response
points: [[1, 2], [2, 3.5], [3, 3], [4, 5], [5, 4.5], [6, 6.2]]
```

## A malformed chart degrades to a code block

```chart
type: bar
nonsense: true
```

Closing paragraph.
