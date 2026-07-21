# Inbox Recommendations — 181 Pending Proposals

> Generated: 2026-07-21 (deterministic heuristic, no LLM)
> Branch: vnext/phase-0
> Source: live SemanticStore (181 pending)

## Summary

| Action | Count | % |
|--------|-------|---|
| 🟢 APPROVE | 87 | 48.1% |
| 🟠 REVIEW | 2 | 1.1% |
| 🔴 REJECT | 92 | 50.8% |
| **TOTAL** | **181** | **100%** |

## Heuristic layers (priority order)

1. **Phase 1.5 reject** — subject fails validator (metric_head, slug, etc.) → REJECT
2. **Soft flag** — MultiEntity + boolean value → REJECT (narrative, not metric)
3. **Weak predicate** — `predicate="is"` or len<3 → REJECT
4. **Metadata subject** — "Research report", "Peer set", "New H-shares", etc. → REJECT
5. **Metric compound** — "CATL market share", "Net cash/share" → REJECT (entity+metric fused)
6. **Entity + qualifier** — "CATL (Q1 2026)" + good predicate → APPROVE
7. **Industry entity** — "TAM ESS", "Chinese battery industry" → APPROVE
8. **Simple acronym/ticker** — CATL, BYD → APPROVE
9. **TitleCase + good predicate** — "Hungary overseas plant" + "investment cost" → APPROVE

## 🟢 APPROVE (87)

### 'BYD' (6 proposals)

- **`business model`** = `vertically integrated (car+batt+charge)`
  - `proposal_id`: `019f7990-bacb-7ca3-bf9f-af7d5c56619b`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker
- **`direction 2026–28`** = `exporting ecosystem to Europe/Turkey`
  - `proposal_id`: `019f7990-bb2a-7580-b725-efbaccc97e5f`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker
- **`main bet`** = `EV using existing battery = main revenue`
  - `proposal_id`: `019f7990-bbd3-7010-adaf-b1d91444ca97`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker
- **`revenue split`** = `cars ~75% + battery/parts ~25%`
  - `proposal_id`: `019f7990-bc2d-7471-be0e-a0b7d95c8140`
  - `domain`: `Finance`
  - `rationale`: clean entity acronym/ticker
- **`strategy`** = `closed ecosystem`
  - `proposal_id`: `019f7990-ba74-76e2-acd7-54d996cb910d`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker
- **`trajectory`** = `shrinking battery share, but growing car share`
  - `proposal_id`: `019f7990-bb80-7fe3-9fb6-0658a064437b`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker

### 'CATL' (33 proposals)

- **`ESS gross margin`** = `~30%`
  - `proposal_id`: `019f795f-0111-7a83-9bd7-c63d971490a0`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`EV Battery gross margin`** = `~24%`
  - `proposal_id`: `019f795f-0024-7a71-ba73-099a3c8e8482`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`EV Battery shipment 2025`** = `541 GWh`
  - `proposal_id`: `019f795f-0070-7110-a782-ea07fbc726bb`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`Other gross margin`** = `~10%`
  - `proposal_id`: `019f795f-01b8-78d0-89ad-bf60b946202a`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`ROIC-WACC spread`** = `+11pp`
  - `proposal_id`: `019f7963-f7b1-7e31-9dfd-47520b4a619a`
  - `domain`: `Finance`
  - `rationale`: clean entity acronym/ticker
- **`WACC`** = `~7.2%`
  - `proposal_id`: `019f7963-f763-7282-a68a-29d10d2d5d56`
  - `domain`: `Finance`
  - `rationale`: clean entity acronym/ticker
- **`all-time high stock price`** = `CNY 469`
  - `proposal_id`: `019f7959-f3ed-7f70-839d-0107102c14d3`
  - `domain`: `financial`
  - `rationale`: clean entity acronym/ticker
- **`base case price target`** = `CNY 412`
  - `proposal_id`: `019f7959-f9f6-7622-83d5-ed5eea4a6720`
  - `domain`: `financial`
  - `rationale`: clean entity acronym/ticker
- **`battery qualification period`** = `2-3 years`
  - `proposal_id`: `019f795f-0448-7720-bbb1-4c0f00f4c8b1`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`biggest threat`** = `Geopolitical fragmentation`
  - `proposal_id`: `019f795f-0499-7940-a497-1b203334ee41`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`bull case price target`** = `CNY 839`
  - `proposal_id`: `019f7959-fa45-7ca3-9f09-65d4fa6e8451`
  - `domain`: `financial`
  - `rationale`: clean entity acronym/ticker
- **`business model`** = `mass volume at gigafactory scale`
  - `proposal_id`: `019f795e-ff35-7421-b22a-bafdb76e9e22`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`completed placement amount in April 2026`** = `$5B`
  - `proposal_id`: `019f7959-f56c-7cf3-bff7-5d20843c4fdd`
  - `domain`: `financial`
  - `rationale`: clean entity acronym/ticker
- **`cost position`** = `lowest cost per unit in the industry`
  - `proposal_id`: `019f795e-ff84-72c3-ba2b-487b784bd9b6`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`current price percentage below all-time high`** = `23%`
  - `proposal_id`: `019f7959-f435-79f3-b302-d606dcb40b19`
  - `domain`: `financial`
  - `rationale`: clean entity acronym/ticker
- **`customers`** = `["Tesla", "BMW", "Mercedes", "VW", "Toyota", "power plant...`
  - `proposal_id`: `019f795e-fee7-79c2-861e-2bf79c14802f`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`customers`** = `multiple OEMs (Tesla, BMW, MB...)`
  - `proposal_id`: `019f7990-b93d-7303-b804-74291b13e9b3`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker
- **`direction 2026–28`** = `dominating multiple chemistries → expanding ESS + sodium-ion`
  - `proposal_id`: `019f7990-b857-78e0-9577-a3fa9413a8c1`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker
- **`expected margin`** = `19-20%`
  - `proposal_id`: `019f7959-fa98-7723-9c6f-5a08fb4d5dfb`
  - `domain`: `financial`
  - `rationale`: clean entity acronym/ticker
- **`implied battery business price per share`** = `CNY 287`
  - `proposal_id`: `019f795a-0036-7602-aa66-10ab9341c0a3`
  - `domain`: `financial`
  - `rationale`: clean entity acronym/ticker
- **`is a cost leader`** = `True`
  - `proposal_id`: `019f799d-04f4-7f70-8a12-55c5b2575175`
  - `domain`: `Business Strategy`
  - `rationale`: clean entity acronym/ticker
- **`is in the best position of the strategic map`** = `True`
  - `proposal_id`: `019f799d-049a-7d53-a3dc-76ed80421f35`
  - `domain`: `Business Strategy`
  - `rationale`: clean entity acronym/ticker
- **`main bet`** = `ESS + sodium-ion = new market`
  - `proposal_id`: `019f7990-b8ed-7f53-8203-cf7d80f9e89a`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker
- **`market share (Jan 2026)`** = `45%`
  - `proposal_id`: `019f799d-0448-78f1-84bb-6df145d5d218`
  - `domain`: `Finance`
  - `rationale`: clean entity acronym/ticker
- **`net cash`** = `¥332B`
  - `proposal_id`: `019f7990-ba20-7351-87cc-8e002acde0eb`
  - `domain`: `Finance`
  - `rationale`: clean entity acronym/ticker
- **`produced chemistries`** = `["LFP", "ternary", "condensed", "sodium-ion"]`
  - `proposal_id`: `019f795f-03f9-7e71-900c-752c20c1d1a3`
  - `domain`: `technology`
  - `rationale`: clean entity acronym/ticker
- **`products`** = `lithium-ion battery cells for EVs and ESS`
  - `proposal_id`: `019f795e-fe74-73b3-82c1-7f5ef47ab485`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`profit growth rate`** = `40%+`
  - `proposal_id`: `019f7959-f3a1-76b0-80f8-01b1e2d397dc`
  - `domain`: `financial`
  - `rationale`: clean entity acronym/ticker
- **`revenue source`** = `battery 100%`
  - `proposal_id`: `019f7990-b98d-7351-baa6-c53d75566512`
  - `domain`: `Finance`
  - `rationale`: clean entity acronym/ticker
- **`shipment scale`** = `661 GWh`
  - `proposal_id`: `019f795f-034b-7e10-aa19-bfc19ec15726`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`shipment size vs competitors`** = `2-3x`
  - `proposal_id`: `019f795f-03a4-7433-ba4d-e5cf1dd3e8b4`
  - `domain`: `business`
  - `rationale`: clean entity acronym/ticker
- **`strategy`** = `cost leader + multi-chemistry`
  - `proposal_id`: `019f7990-b80b-7ee1-b941-88f78ffcbf4f`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker
- **`trajectory`** = `39→45%`
  - `proposal_id`: `019f7990-b8a1-7dc1-93d6-0fbc48a6940a`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker

### 'CATL (2026 funding)' (1 proposal)

- **`HK placement amount`** = `$5B`
  - `proposal_id`: `019f7963-f863-7562-9719-7dcb570e6a2b`
  - `domain`: `Finance`
  - `rationale`: entity+qualifier + good predicate

### 'CATL (Q1 2026)' (1 proposal)

- **`revenue growth`** = `50%+`
  - `proposal_id`: `019f7963-f8af-7981-aa3c-c23bebbcf426`
  - `domain`: `Finance`
  - `rationale`: entity+qualifier + good predicate

### 'CATL ESS business' (1 proposal)

- **`expected annual growth rate`** = `>35%`
  - `proposal_id`: `019f7959-f619-72f0-97e8-78ec557832ec`
  - `domain`: `business`
  - `rationale`: capitalized subject + meaningful predicate

### 'CATL EV battery business' (1 proposal)

- **`expected annual growth rate`** = `12-18%`
  - `proposal_id`: `019f7959-f5bb-7b00-bf82-7c26d31cff13`
  - `domain`: `business`
  - `rationale`: capitalized subject + meaningful predicate

### 'CATL shipment (2025)' (1 proposal)

- **`volume`** = `661 GWh`
  - `proposal_id`: `019f7963-fcc9-7603-a85e-a10b34a6431a`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'CATL shipment (2030)' (1 proposal)

- **`volume`** = `~1,400–1,600 GWh`
  - `proposal_id`: `019f7963-fd14-7c81-8178-ce2580ea69ac`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'CATL shipment 2025' (1 proposal)

- **`actual volume is`** = `661 GWh`
  - `proposal_id`: `019f799b-0aab-70f1-8e17-1da2d36cae9a`
  - `domain`: `corporate_performance`
  - `rationale`: entity+qualifier + good predicate

### 'CATL shipment 2030E' (1 proposal)

- **`estimated volume is`** = `≈1,561 GWh`
  - `proposal_id`: `019f799b-0b04-7ad2-b10e-688f349e2b00`
  - `domain`: `corporate_performance`
  - `rationale`: capitalized subject + meaningful predicate

### 'China (CATL+BYD)' (1 proposal)

- **`strategic timeframe`** = `today`
  - `proposal_id`: `019f7990-bf67-7f73-a45b-a3e1859c0767`
  - `domain`: `Business`
  - `rationale`: entity+qualifier + good predicate

### 'Chinese battery industry' (1 proposal)

- **`overcapacity`** = `~2x demand`
  - `proposal_id`: `019f795f-058f-7981-a8fb-7504e4d164df`
  - `domain`: `industry`
  - `rationale`: industry-level entity + meaningful predicate

### 'Debrecen production start' (1 proposal)

- **`is scheduled for`** = `Q1 2026`
  - `proposal_id`: `019f798d-54be-7372-a7c3-2575640087e0`
  - `domain`: `manufacturing`
  - `rationale`: capitalized subject + meaningful predicate

### 'ESS' (2 proposals)

- **`TAM 2030`** = `~1,000+ GWh`
  - `proposal_id`: `019f795f-0681-78a1-bbb0-81297d8fffc3`
  - `domain`: `industry`
  - `rationale`: clean entity acronym/ticker
- **`growth rate per year`** = `>50%`
  - `proposal_id`: `019f7963-f40c-7141-9f76-781357e2380b`
  - `domain`: `Business`
  - `rationale`: clean entity acronym/ticker

### 'EV Battery' (1 proposal)

- **`TAM 2030`** = `~3,000+ GWh`
  - `proposal_id`: `019f795f-0632-7c52-a5f3-de7fc1d7f378`
  - `domain`: `industry`
  - `rationale`: capitalized subject + meaningful predicate

### 'Global' (2 proposals)

- **`EV penetration rate`** = `~20%`
  - `proposal_id`: `019f795f-04ec-7a92-b8d1-cd6353520d27`
  - `domain`: `industry`
  - `rationale`: capitalized subject + meaningful predicate
- **`EV penetration target 2030`** = `40-50%`
  - `proposal_id`: `019f795f-053d-7fb3-a240-f618aea7b55a`
  - `domain`: `industry`
  - `rationale`: capitalized subject + meaningful predicate

### 'Hungary overseas plant' (2 proposals)

- **`capacity`** = `100 GWh`
  - `proposal_id`: `019f7963-f539-7071-9a86-5b541095a860`
  - `domain`: `Production`
  - `rationale`: capitalized subject + meaningful predicate
- **`investment cost`** = `€7.3B`
  - `proposal_id`: `019f7963-f4ed-7983-a52e-71f68e76437c`
  - `domain`: `Finance`
  - `rationale`: capitalized subject + meaningful predicate

### 'Korea (LG+Samsung)' (1 proposal)

- **`strategic timeframe`** = `future`
  - `proposal_id`: `019f7990-bfc9-70f1-a12e-52d927a2e37f`
  - `domain`: `Business`
  - `rationale`: entity+qualifier + good predicate

### 'LG Energy' (4 proposals)

- **`capex cut`** = `20–30%`
  - `proposal_id`: `019f7990-bd53-74e1-8424-7b11b4159802`
  - `domain`: `Finance`
  - `rationale`: capitalized subject + meaningful predicate
- **`direction 2026–28`** = `NCM legacy → turning to LFP`
  - `proposal_id`: `019f7990-bcef-7221-8a31-69e841fa79e0`
  - `domain`: `Business`
  - `rationale`: capitalized subject + meaningful predicate
- **`strategy`** = `long-term technology bet`
  - `proposal_id`: `019f7990-bc8a-7cb1-9811-f113a25fe218`
  - `domain`: `Business`
  - `rationale`: capitalized subject + meaningful predicate
- **`trajectory Q1'26`** = `−15%`
  - `proposal_id`: `019f7990-bda3-7383-8cce-b65d6e6d4f76`
  - `domain`: `Business`
  - `rationale`: capitalized subject + meaningful predicate

### 'Profit' (1 proposal)

- **`growth rate`** = `45%`
  - `proposal_id`: `019f7996-94f0-7b32-be8e-93ea72da47d8`
  - `domain`: `corporate_performance`
  - `rationale`: capitalized subject + meaningful predicate

### 'Samsung SDI' (4 proposals)

- **`business model`** = `Prismatic + tech differentiated`
  - `proposal_id`: `019f7990-be5f-72d1-a1eb-6c9d8e143c17`
  - `domain`: `Business`
  - `rationale`: capitalized subject + meaningful predicate
- **`direction 2026–28`** = `betting all-solid-state 2027`
  - `proposal_id`: `019f7990-beba-7a70-8802-99d8577d0107`
  - `domain`: `Business`
  - `rationale`: capitalized subject + meaningful predicate
- **`strategy`** = `long-term technology bet`
  - `proposal_id`: `019f7990-bdf7-7681-923e-11770416b538`
  - `domain`: `Business`
  - `rationale`: capitalized subject + meaningful predicate
- **`trajectory`** = `heavily receding (loss, ranking drop)`
  - `proposal_id`: `019f7990-bf0f-73f2-8b5f-ec93db54b72f`
  - `domain`: `Business`
  - `rationale`: capitalized subject + meaningful predicate

### 'Sodium-ion (Naxtra)' (2 proposals)

- **`cost comparison to Li`** = `cost ครึ่ง Li`
  - `proposal_id`: `019f7963-f455-7452-9928-1a9a1311b5dc`
  - `domain`: `Technology`
  - `rationale`: entity+qualifier + good predicate
- **`full production start year`** = `2026`
  - `proposal_id`: `019f7963-f4a2-7552-9a65-43f0362c8b6a`
  - `domain`: `Production`
  - `rationale`: entity+qualifier + good predicate

### 'Sodium-ion failure' (1 proposal)

- **`growth reduction is`** = `~15%`
  - `proposal_id`: `019f799b-0bb8-7ca1-9d72-aaf34bae2769`
  - `domain`: `risk`
  - `rationale`: capitalized subject + meaningful predicate

### 'Sodium-ion full success' (1 proposal)

- **`new revenue is`** = `¥150B`
  - `proposal_id`: `019f799b-0b5d-75a2-9c0d-09a6fbc98a94`
  - `domain`: `finance`
  - `rationale`: capitalized subject + meaningful predicate

### 'Spain JV' (2 proposals)

- **`investment cost`** = `€4.1B`
  - `proposal_id`: `019f7963-f582-75c3-a4cf-ea52465da20c`
  - `domain`: `Finance`
  - `rationale`: capitalized subject + meaningful predicate
- **`partner company`** = `Stellantis`
  - `proposal_id`: `019f7963-f5cf-79b3-a29a-71f83db4cc32`
  - `domain`: `Business`
  - `rationale`: capitalized subject + meaningful predicate

### 'TAM Datacenter ESS' (1 proposal)

- **`CAGR`** = `~80%+`
  - `proposal_id`: `019f7963-fba0-7ee3-8bc5-83802c79ce9d`
  - `domain`: `Market`
  - `rationale`: industry-level entity + meaningful predicate

### 'TAM Datacenter ESS (2025)' (1 proposal)

- **`volume`** = `~10 GWh`
  - `proposal_id`: `019f7963-fb0f-7922-beb6-075346633689`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'TAM Datacenter ESS (2030)' (1 proposal)

- **`volume`** = `~300 GWh`
  - `proposal_id`: `019f7963-fb57-7f01-822b-eb749a6b4163`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'TAM ESS' (1 proposal)

- **`CAGR`** = `~25–30%`
  - `proposal_id`: `019f7963-fac2-7873-bdd7-a7767924d84a`
  - `domain`: `Market`
  - `rationale`: industry-level entity + meaningful predicate

### 'TAM ESS (2025)' (1 proposal)

- **`volume`** = `~550 GWh`
  - `proposal_id`: `019f7963-fa2a-71e3-9c95-8e753bbf4f2b`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'TAM ESS (2030)' (1 proposal)

- **`volume`** = `~1,000–1,500 GWh`
  - `proposal_id`: `019f7963-fa74-7970-bf3d-321fda8cada6`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'TAM EV battery' (1 proposal)

- **`CAGR`** = `~20%`
  - `proposal_id`: `019f7963-f9db-78c1-ac40-482789a4497f`
  - `domain`: `Market`
  - `rationale`: industry-level entity + meaningful predicate

### 'TAM EV battery (2025)' (1 proposal)

- **`volume`** = `~1,200 GWh`
  - `proposal_id`: `019f7963-f948-75b0-be6b-52584efd0676`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'TAM EV battery (2030)' (1 proposal)

- **`volume`** = `~3,000 GWh`
  - `proposal_id`: `019f7963-f995-7410-a224-193e4e8bdd2d`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'Total TAM' (1 proposal)

- **`CAGR`** = `~22%`
  - `proposal_id`: `019f7963-fc81-7593-bb30-15ca5e0e9634`
  - `domain`: `Market`
  - `rationale`: industry-level entity + meaningful predicate

### 'Total TAM (2025)' (1 proposal)

- **`volume`** = `~1,750 GWh`
  - `proposal_id`: `019f7963-fbf1-7d20-bcbd-4a5b760fd816`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'Total TAM (2030)' (1 proposal)

- **`volume`** = `~4,000–4,500 GWh`
  - `proposal_id`: `019f7963-fc38-7953-b14a-9053859df355`
  - `domain`: `Market`
  - `rationale`: entity+qualifier + good predicate

### 'US' (1 proposal)

- **`tax on Chinese batteries`** = `100%+`
  - `proposal_id`: `019f795f-05e1-7e73-b892-8e83d469a9af`
  - `domain`: `economics`
  - `rationale`: clean entity acronym/ticker

### 'Volume' (1 proposal)

- **`relative to average`** = `0.8x`
  - `proposal_id`: `019f7996-97be-7832-95b9-b83eb2bac46d`
  - `domain`: `market_data`
  - `rationale`: capitalized subject + meaningful predicate

## 🟠 REVIEW (2)

### 'China challengers (CALB, EVE)' (2 proposals)

- **`cannot withstand price cuts for long`** = `True`
  - `proposal_id`: `019f799d-0392-7e02-af1a-e68326c6a800`
  - `domain`: `Business Strategy`
  - `rationale`: split needed (shape=MultiEntity)
- **`have no cash`** = `True`
  - `proposal_id`: `019f799d-03e8-75a1-af2d-f56521e696a5`
  - `domain`: `Finance`
  - `rationale`: split needed (shape=MultiEntity)

## 🔴 REJECT (92)

### '50% Fib yearly level' (1 proposal)

- **`is`** = `¥351.5`
  - `proposal_id`: `019f7996-96da-7482-8e7e-ac1fdf69b8c7`
  - `domain`: `technical_analysis`
  - `rationale`: Phase 1.5 (shape=NumberLed)

### 'ASEAN/Latin/MEA revenue share projection' (1 proposal)

- **`is`** = `~10% → 20%`
  - `proposal_id`: `019f798d-56b7-71d2-b1ff-d21f22e27670`
  - `domain`: `business`
  - `rationale`: predicate 'is' vague

### 'Analyst recommendations' (1 proposal)

- **`distribution`** = `27 buy/strong-buy, 0 hold, 1 strong-sell`
  - `proposal_id`: `019f7996-9366-7c63-bafb-4ff9e7890fc9`
  - `domain`: `financial_analysis`
  - `rationale`: metadata subject (not entity claim)

### 'BYD market share' (2 proposals)

- **`percentage is`** = `16%`
  - `proposal_id`: `019f799b-0d46-76c0-8372-cf5e03a61793`
  - `domain`: `competition`
  - `rationale`: entity+metric fused subject
- **`rank is`** = `#2`
  - `proposal_id`: `019f799b-0ccf-7950-b4c8-7e93a15b4049`
  - `domain`: `competition`
  - `rationale`: entity+metric fused subject

### 'Beta' (2 proposals)

- **`assumed value`** = `0.95`
  - `proposal_id`: `019f7998-5b15-77d0-b7e2-27e49670b262`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_single)
- **`is`** = `0.95`
  - `proposal_id`: `019f79a1-f6ce-73f2-b4ff-cb21667b66ff`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_single)

### 'CATL 2025 Annual Report' (1 proposal)

- **`date`** = `10 Mar 2026`
  - `proposal_id`: `019f7998-5efa-71e3-8b91-fe341ec0eff4`
  - `domain`: `finance`
  - `rationale`: metadata subject (not entity claim)

### 'CATL CapEx (2026)' (1 proposal)

- **`projection`** = `¥50–65B`
  - `proposal_id`: `019f7963-f800-7fc3-9ce2-fecb37852c9b`
  - `domain`: `Finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'CATL Q1 2026 results' (1 proposal)

- **`date`** = `15 Apr 2026`
  - `proposal_id`: `019f7998-5f51-7ef1-89f3-15cec360f826`
  - `domain`: `finance`
  - `rationale`: metadata subject (not entity claim)

### 'CATL cost' (1 proposal)

- **`reduction rate per year`** = `15%`
  - `proposal_id`: `019f7963-f8fb-7ba3-aaea-4a861ba75545`
  - `domain`: `Finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'CATL market share' (2 proposals)

- **`percentage is`** = `39%`
  - `proposal_id`: `019f799b-0c77-7893-97ae-87f05cd75dcd`
  - `domain`: `competition`
  - `rationale`: entity+metric fused subject
- **`rank is`** = `#1`
  - `proposal_id`: `019f799b-0c1f-7fb2-b0af-61dad4ce49f6`
  - `domain`: `competition`
  - `rationale`: entity+metric fused subject

### 'Cash' (1 proposal)

- **`is`** = `¥451.5B`
  - `proposal_id`: `019f79a1-fa7e-7991-8c7c-92a4530c542c`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Cash (post-placement)' (1 proposal)

- **`value`** = `¥451.5B`
  - `proposal_id`: `019f7998-5c5f-70f3-a715-b45e1eb43a97`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Chart capture date' (1 proposal)

- **`is`** = `8 Jul 2026`
  - `proposal_id`: `019f7996-9646-78a0-aeef-39e4f3776ab9`
  - `domain`: `metadata`
  - `rationale`: predicate 'is' vague

### 'China CAGR' (1 proposal)

- **`is approximately`** = `~10%`
  - `proposal_id`: `019f798d-5333-7033-bb96-692b6965a765`
  - `domain`: `business`
  - `rationale`: Phase 1.5 (metric_head)

### 'China revenue share' (1 proposal)

- **`is approximately`** = `~50%`
  - `proposal_id`: `019f798d-52e6-7173-8cc9-b294bf8d3aea`
  - `domain`: `business`
  - `rationale`: entity+metric fused subject

### 'Cost of equity' (1 proposal)

- **`is`** = `7.54%`
  - `proposal_id`: `019f79a1-f726-7193-8313-89cee476d9bc`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Country risk premium' (1 proposal)

- **`assumed value`** = `0.6%`
  - `proposal_id`: `019f7998-5a68-73d2-acad-d9c142dd9ce1`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Current case price' (1 proposal)

- **`is`** = `¥361`
  - `proposal_id`: `019f79a1-fc5d-79d3-8fa7-32ef4cd3e1fd`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'DCF price per share' (1 proposal)

- **`is`** = `¥447.6`
  - `proposal_id`: `019f79a1-fbfc-7770-855c-6ca2189f618b`
  - `domain`: `finance`
  - `rationale`: predicate 'is' vague

### 'Debrecen factory capacity' (1 proposal)

- **`is`** = `100 GWh`
  - `proposal_id`: `019f798d-5467-7a01-bd30-c81ab0a993e6`
  - `domain`: `manufacturing`
  - `rationale`: predicate 'is' vague

### 'Debrecen factory investment' (1 proposal)

- **`is`** = `€7.3B`
  - `proposal_id`: `019f798d-541b-7563-a0d4-3bfd16f1a900`
  - `domain`: `business`
  - `rationale`: predicate 'is' vague

### 'Debt' (1 proposal)

- **`is`** = `¥119.7B`
  - `proposal_id`: `019f79a1-fad7-7943-a176-bd5b16bddbf8`
  - `domain`: `finance`
  - `rationale`: predicate 'is' vague

### 'ESS growth contribution' (2 proposals)

- **`percentage share is`** = `35–40%`
  - `proposal_id`: `019f799b-07a9-7641-8a42-ffb5ae98eb70`
  - `domain`: `energy_storage`
  - `rationale`: entity+metric fused subject
- **`volume is`** = `300–400 GWh`
  - `proposal_id`: `019f799b-0753-74a2-9ec2-eb660edd9ac9`
  - `domain`: `energy_storage`
  - `rationale`: entity+metric fused subject

### 'EU CAGR' (1 proposal)

- **`is`** = `~30%+`
  - `proposal_id`: `019f798d-53d1-7893-b114-0653142fc735`
  - `domain`: `business`
  - `rationale`: Phase 1.5 (metric_head)

### 'EU revenue share projection' (1 proposal)

- **`is`** = `~20% → 30%`
  - `proposal_id`: `019f798d-5386-7130-97a0-8a5ae518d05b`
  - `domain`: `business`
  - `rationale`: predicate 'is' vague

### 'EV battery growth contribution' (2 proposals)

- **`percentage share is`** = `35–40%`
  - `proposal_id`: `019f799b-086b-7dd0-93d0-83aa8d8f2783`
  - `domain`: `ev_battery`
  - `rationale`: entity+metric fused subject
- **`volume is`** = `300–400 GWh`
  - `proposal_id`: `019f799b-080c-7641-968e-5fe81f7c4192`
  - `domain`: `ev_battery`
  - `rationale`: entity+metric fused subject

### 'Emerging markets growth contribution' (2 proposals)

- **`percentage share is`** = `6–10%`
  - `proposal_id`: `019f799b-09f4-7d23-8fe4-ad353840c71a`
  - `domain`: `market_analysis`
  - `rationale`: entity+metric fused subject
- **`volume is`** = `50–100 GWh`
  - `proposal_id`: `019f799b-0994-7002-8be8-8b536e8982df`
  - `domain`: `market_analysis`
  - `rationale`: entity+metric fused subject

### 'Enterprise Value' (1 proposal)

- **`is`** = `¥1,669B`
  - `proposal_id`: `019f79a1-f9b7-7242-9ac3-6b545d35c6fe`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Entry zone' (1 proposal)

- **`is`** = `¥340–360`
  - `proposal_id`: `019f7996-980a-7cd3-9817-2b9096bde2b9`
  - `domain`: `trading_strategy`
  - `rationale`: predicate 'is' vague

### 'Equity Value' (1 proposal)

- **`is`** = `¥2,000.8B`
  - `proposal_id`: `019f79a1-fb37-7082-81c5-8511e7996993`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Equity risk premium' (2 proposals)

- **`assumed value`** = `6.10%`
  - `proposal_id`: `019f7998-5abb-7fe2-b315-ed3ec7ea809a`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)
- **`is`** = `6.10%`
  - `proposal_id`: `019f79a1-f670-7353-985e-275295bc7bc5`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Exchange' (1 proposal)

- **`is`** = `SZSE`
  - `proposal_id`: `019f7996-95fc-7df0-948a-32171b9a5291`
  - `domain`: `market_data`
  - `rationale`: predicate 'is' vague

### 'Growth source 2026-2028' (1 proposal)

- **`is`** = `EU ramp`
  - `proposal_id`: `019f798d-524b-7333-8aae-674758ce9aa0`
  - `domain`: `business`
  - `rationale`: predicate 'is' vague

### 'Growth source 2028+' (1 proposal)

- **`is`** = `emerging markets`
  - `proposal_id`: `019f798d-5299-7312-962c-2a7348ef4cd2`
  - `domain`: `business`
  - `rationale`: predicate 'is' vague

### 'HK listing premium' (1 proposal)

- **`value`** = `~85%`
  - `proposal_id`: `019f7998-5dae-75d3-abc9-63b70d488389`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'HK target price' (1 proposal)

- **`is approximately`** = `HK$664`
  - `proposal_id`: `019f7996-9403-7820-b911-5a9216108754`
  - `domain`: `financial_analysis`
  - `rationale`: Phase 1.5 (metric_head)

### 'Median target price' (1 proposal)

- **`is approximately`** = `¥560`
  - `proposal_id`: `019f7996-93ba-7113-b264-b83bc41d8cd8`
  - `domain`: `financial_analysis`
  - `rationale`: Phase 1.5 (metric_head)

### 'Net Cash' (1 proposal)

- **`is`** = `¥331.8B`
  - `proposal_id`: `019f79a1-fa1f-7972-89a4-de8f316f2e75`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Net cash/share' (1 proposal)

- **`value`** = `¥74.2`
  - `proposal_id`: `019f7998-5d07-7502-839f-7fbeef7a5878`
  - `domain`: `finance`
  - `rationale`: entity+metric fused subject

### 'New H-shares' (2 proposals)

- **`count`** = `62.385M`
  - `proposal_id`: `019f7998-5e58-7101-a998-11e65cb26bba`
  - `domain`: `finance`
  - `rationale`: metadata subject (not entity claim)
- **`date`** = `28 Apr 2026`
  - `proposal_id`: `019f7998-5ea9-7710-b562-1ba4e4cdbce0`
  - `domain`: `finance`
  - `rationale`: metadata subject (not entity claim)

### 'Overseas Capex' (1 proposal)

- **`annual projection`** = `¥50–65B`
  - `proposal_id`: `019f7998-5e03-7f93-b4ea-cef28101fce5`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'PV(FCFF)' (1 proposal)

- **`is`** = `¥211.5B`
  - `proposal_id`: `019f79a1-f907-7df2-971b-8fcdb0abf02d`
  - `domain`: `finance`
  - `rationale`: predicate 'is' vague

### 'PV(TV)' (1 proposal)

- **`is`** = `¥1,457.5B`
  - `proposal_id`: `019f79a1-f961-7213-965a-89105a030ef3`
  - `domain`: `finance`
  - `rationale`: predicate 'is' vague

### 'Peer set' (1 proposal)

- **`size`** = `5`
  - `proposal_id`: `019f7998-5c11-7b11-8d93-979bb8a9044f`
  - `domain`: `finance`
  - `rationale`: metadata subject (not entity claim)

### 'Price' (1 proposal)

- **`decrease from ATH`** = `23%`
  - `proposal_id`: `019f7996-944e-7c30-a898-afc68395d90f`
  - `domain`: `market_data`
  - `rationale`: Phase 1.5 (metric_head)

### 'Q1 2026 performance' (1 proposal)

- **`beat estimate by`** = `33%`
  - `proposal_id`: `019f7996-94a0-79a2-bfcb-9aab0cc90d32`
  - `domain`: `corporate_performance`
  - `rationale`: metadata subject (not entity claim)

### 'Q2 2026 earnings report' (1 proposal)

- **`scheduled date`** = `25 Jul 2026`
  - `proposal_id`: `019f7996-9540-7721-919a-32b498da939c`
  - `domain`: `corporate_events`
  - `rationale`: metadata subject (not entity claim)

### 'RSI' (1 proposal)

- **`is`** = `33.6`
  - `proposal_id`: `019f7996-9774-7cf3-b4e5-57b1ad57ad7b`
  - `domain`: `technical_analysis`
  - `rationale`: predicate 'is' vague

### 'Recent low' (1 proposal)

- **`is`** = `¥361`
  - `proposal_id`: `019f7996-9727-78f2-83a7-eaab27e334cc`
  - `domain`: `market_data`
  - `rationale`: predicate 'is' vague

### 'Research report' (1 proposal)

- **`creation date`** = `9 Jul 2026`
  - `proposal_id`: `019f7998-59ba-7860-9a76-63d408cfa391`
  - `domain`: `financial_research`
  - `rationale`: metadata subject (not entity claim)

### 'Revenue growth path' (1 proposal)

- **`is`** = `18%→14%→11%→8%→5.5%`
  - `proposal_id`: `019f79a1-f7e4-7020-88da-8f27baca3b87`
  - `domain`: `finance`
  - `rationale`: predicate 'is' vague

### 'Risk-free rate' (2 proposals)

- **`assumed value`** = `1.75%`
  - `proposal_id`: `019f7998-5a14-76a0-997b-d1f9b336b144`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)
- **`is`** = `1.75%`
  - `proposal_id`: `019f79a1-f614-7600-b917-ded8b5d798d9`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Shares (post-placement)' (1 proposal)

- **`value`** = `4.470B`
  - `proposal_id`: `019f7998-5cb3-7aa3-86e2-acb94405deff`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Shares outstanding' (1 proposal)

- **`is`** = `4.470B`
  - `proposal_id`: `019f79a1-fb9c-7781-8c60-1e276d4e3267`
  - `domain`: `finance`
  - `rationale`: predicate 'is' vague

### 'Shipment CAGR' (1 proposal)

- **`is projected to be`** = `17–19%`
  - `proposal_id`: `019f799b-0d9f-7de0-8f69-fbf433f1140e`
  - `domain`: `corporate_performance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Sodium-ion growth contribution' (2 proposals)

- **`percentage share is`** = `12–15%`
  - `proposal_id`: `019f799b-092e-7242-8263-b958e6518c2b`
  - `domain`: `battery_tech`
  - `rationale`: entity+metric fused subject
- **`volume is`** = `100–150 GWh`
  - `proposal_id`: `019f799b-08c9-7f53-b2e8-a3b4eee88487`
  - `domain`: `battery_tech`
  - `rationale`: entity+metric fused subject

### 'Spain JV investment' (1 proposal)

- **`is`** = `€4.1B`
  - `proposal_id`: `019f798d-5512-7cf3-a2ea-69354b3a7946`
  - `domain`: `business`
  - `rationale`: predicate 'is' vague

### 'Spain JV partner' (1 proposal)

- **`is`** = `Stellantis`
  - `proposal_id`: `019f798d-5567-7c52-9b85-f18d1a5989fa`
  - `domain`: `business`
  - `rationale`: predicate 'is' vague

### 'Stock ticker' (1 proposal)

- **`is`** = `300750`
  - `proposal_id`: `019f7996-958b-7801-9c6a-a4ad0bb909e1`
  - `domain`: `market_data`
  - `rationale`: predicate 'is' vague

### 'Stop loss' (1 proposal)

- **`is`** = `¥325`
  - `proposal_id`: `019f7996-9858-7bf2-875e-6cda6a2881eb`
  - `domain`: `trading_strategy`
  - `rationale`: predicate 'is' vague

### 'TAM growth' (1 proposal)

- **`annual rate is`** = `22%/year`
  - `proposal_id`: `019f799b-0e08-7b71-9be2-80cadf272949`
  - `domain`: `market_analysis`
  - `rationale`: Phase 1.5 (metric_head)

### 'Target 1' (1 proposal)

- **`is`** = `¥400`
  - `proposal_id`: `019f7996-98ac-7631-9efb-0b95c6b2768f`
  - `domain`: `trading_strategy`
  - `rationale`: predicate 'is' vague

### 'Tax rate' (1 proposal)

- **`assumed value`** = `14.2%`
  - `proposal_id`: `019f7998-5b6e-7fb1-bd5e-9cbd9b913779`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Technical invalidation level' (1 proposal)

- **`value`** = `¥325`
  - `proposal_id`: `019f7998-5d56-7631-8f10-7b8fa459378f`
  - `domain`: `finance`
  - `rationale`: entity+metric fused subject

### 'Terminal growth' (2 proposals)

- **`assumed value`** = `3.0%`
  - `proposal_id`: `019f7998-5bc1-7291-b5c2-88c8bb2554e3`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)
- **`is`** = `3.0%`
  - `proposal_id`: `019f79a1-f89e-7793-b619-d93127b7ae08`
  - `domain`: `finance`
  - `rationale`: Phase 1.5 (metric_head)

### 'Total growth' (1 proposal)

- **`volume is`** = `~750–1,000 GWh`
  - `proposal_id`: `019f799b-0a55-7382-a952-2b9cda5879f7`
  - `domain`: `market_analysis`
  - `rationale`: Phase 1.5 (metric_head)

### 'US eligibility for EV credit' (1 proposal)

- **`is`** = `not eligible for $7,500 credit`
  - `proposal_id`: `019f798d-566a-7852-9b11-3a422cbf6194`
  - `domain`: `business`
  - `rationale`: predicate 'is' vague

### 'US revenue share projection' (1 proposal)

- **`is`** = `~5% → low negative/closed`
  - `proposal_id`: `019f798d-55c1-7a33-9eb0-65fb3103e72f`
  - `domain`: `business`
  - `rationale`: predicate 'is' vague

### 'US tariff rate' (1 proposal)

- **`is`** = `100%+`
  - `proposal_id`: `019f798d-561b-70b2-9f1a-d0608aac2b44`
  - `domain`: `business`
  - `rationale`: Phase 1.5 (metric_head)

### 'WACC' (1 proposal)

- **`is`** = `7.17%`
  - `proposal_id`: `019f79a1-f78b-7bf1-9e2d-cade5418312b`
  - `domain`: `finance`
  - `rationale`: predicate 'is' vague

### 'Weekly SMA100' (1 proposal)

- **`is`** = `¥304`
  - `proposal_id`: `019f7996-968f-77e3-a058-3a5fe58fc0bf`
  - `domain`: `technical_analysis`
  - `rationale`: predicate 'is' vague

### 'freight-china-soybean-macro-2026-07' (1 proposal)

- **`has_macro_data`** = `macro_data_freight_china_2026`
  - `proposal_id`: `019f7df1-346e-7113-82be-962cd6a0e067`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

### 'psl-fair-value-2026-07' (1 proposal)

- **`has_fair_value`** = `psl_fv_10_4_to_15_5_THB_conditional`
  - `proposal_id`: `019f7dc9-a1c6-7f11-b424-fe621d5e95a0`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

### 'psl-rcl-set-factsheet-2026-07' (1 proposal)

- **`has_primary_data`** = `primary_set_data_psl_rcl_2026`
  - `proposal_id`: `019f7ded-d467-7353-811f-1cf3f8d99840`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

### 'simandou-chokepoints-2026-07' (1 proposal)

- **`has_geopolitical_data`** = `simandou_chokepoints_2026`
  - `proposal_id`: `019f7df3-d409-7f70-9b70-637adc6af5d9`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

### 'simandou-first-shipment' (1 proposal)

- **`has_fact`** = `first_shipment_jan_2026_china`
  - `proposal_id`: `019f7dc0-eae9-7801-90ac-b1436d10dd30`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

### 'thai-shipping-bf-report-2026-07' (1 proposal)

- **`produced_deliverable`** = `bf_report_delivered_76KB`
  - `proposal_id`: `019f7dd7-7c5f-7c83-ad92-a4152e766784`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

### 'thai-shipping-catalyst-research-2026-07' (1 proposal)

- **`requires`** = `catalyst_theme_ton_mile_focus`
  - `proposal_id`: `019f7db5-e617-72f0-b2d2-6d55498c8560`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

### 'thai-shipping-catalyst-thesis-2026' (1 proposal)

- **`has_thesis`** = `psl_not_just_thai_discount_conditional_thesis`
  - `proposal_id`: `019f7dc6-4859-7ca2-9a14-8c5bfa96edab`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

### 'thai-shipping-stocks-2026-07-20' (1 proposal)

- **`has_market_state`** = `thai_shipping_local_rerating_july_2026`
  - `proposal_id`: `019f7dad-0de4-7f71-870e-973f9091f991`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

### 'thai-shipping-vs-peers-2026-07' (1 proposal)

- **`has_valuation`** = `thai_discount_fleet_cycle_evidence`
  - `proposal_id`: `019f7dc0-eb53-7dd3-af0b-d4bce61cec52`
  - `domain`: `shipping`
  - `rationale`: Phase 1.5 (shape=Slug)

---

## How to execute (after approval)

```bash
# For each APPROVE proposal:
curl -b cookies.txt -X POST http://127.0.0.1:8080/api/v1/inbox/{proposal_id}/approve

# For each REJECT proposal:
curl -b cookies.txt -X POST http://127.0.0.1:8080/api/v1/inbox/{proposal_id}/reject
```

Or tell the agent "execute approved" and it will bulk-call the API.
