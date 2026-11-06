#!/usr/bin/env python3
"""Generate the shared spike dataset as CSV: sales fact (~1M rows) + products dim (500 rows)."""
import csv, random, os

random.seed(42)
HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "data")
os.makedirs(DATA, exist_ok=True)

REGIONS = ["North", "South", "East", "West", "Central"]
CATEGORIES = ["Hardware", "Software", "Services", "Accessories"]
BRANDS = ["Acme", "Globex", "Initech", "Umbrella", "Stark", "Wayne", "Wonka", "Hooli"]

with open(os.path.join(DATA, "products.csv"), "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["product_id", "category", "product_name", "brand"])
    for pid in range(1, 501):
        cat = CATEGORIES[pid % len(CATEGORIES)]
        w.writerow([pid, cat, f"Product-{pid:03d}", BRANDS[pid % len(BRANDS)]])

rows = 1_000_000
with open(os.path.join(DATA, "sales.csv"), "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(["order_id", "order_date", "customer_id", "product_id", "region", "quantity", "unit_price", "discount"])
    for oid in range(1, rows + 1):
        y = random.choice([2023, 2024, 2024, 2025])
        m = random.randint(1, 12)
        d = random.randint(1, 28)
        w.writerow([
            oid,
            f"{y:04d}-{m:02d}-{d:02d}",
            random.randint(1, 50_000),
            random.randint(1, 500),
            random.choice(REGIONS),
            random.randint(1, 20),
            round(random.uniform(5.0, 500.0), 2),
            random.choice([0.0, 0.0, 0.05, 0.1, 0.15, 0.2]),
        ])
print("wrote", rows, "sales rows and 500 products to", DATA)
