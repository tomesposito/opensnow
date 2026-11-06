#!/usr/bin/env python3
"""PyIceberg end-to-end spike: create v2, write, append, delete/update, time travel, small files."""
import os, sys, traceback
import pyarrow as pa
from pyiceberg.catalog import load_catalog
from pyiceberg.schema import Schema
from pyiceberg.types import NestedField, LongType, StringType, DoubleType
from pyiceberg.partitioning import PartitionSpec, PartitionField
from pyiceberg.transforms import IdentityTransform
from pyiceberg.expressions import EqualTo

BASE = os.path.dirname(os.path.abspath(__file__))
WH = os.path.join(BASE, "warehouse")
os.makedirs(WH, exist_ok=True)

log = []
def step(name, fn):
    try:
        r = fn()
        log.append(f"OK   {name}: {r}")
        print(f"OK   {name}: {r}", flush=True)
        return r
    except Exception as e:
        log.append(f"FAIL {name}: {type(e).__name__}: {e}")
        print(f"FAIL {name}: {type(e).__name__}: {e}", flush=True)
        traceback.print_exc()
        return None

catalog = load_catalog("spike", **{
    "type": "sql",
    "uri": f"sqlite:///{BASE}/catalog.db",
    "warehouse": f"file://{WH}",
})
step("create_namespace", lambda: (catalog.create_namespace("db"), "db")[1])

schema = Schema(
    NestedField(1, "id", LongType(), required=True),
    NestedField(2, "city", StringType()),
    NestedField(3, "amount", DoubleType()),
    identifier_field_ids={1},
)
spec = PartitionSpec(PartitionField(source_id=2, field_id=1000, transform=IdentityTransform(), name="city"))

ARROW_SCHEMA = pa.schema([
    pa.field("id", pa.int64(), nullable=False),
    pa.field("city", pa.string()),
    pa.field("amount", pa.float64()),
])
def make_batch(start, n, city="NYC", amt=1.5):
    return pa.table({
        "id": pa.array(range(start, start + n), type=pa.int64()),
        "city": pa.array([city] * n),
        "amount": pa.array([amt] * n, type=pa.float64()),
    }, schema=ARROW_SCHEMA)

tbl = step("create_table_v2", lambda: catalog.create_table("db.events", schema=schema, partition_spec=spec, properties={"format-version": "2"}))
if tbl is None:
    sys.exit(1)
step("format_version", lambda: tbl.metadata.format_version)

step("write_100k", lambda: (tbl.append(make_batch(0, 100_000)), tbl.scan().count())[1])
snap1 = step("snapshot_after_write", lambda: tbl.current_snapshot().snapshot_id)
step("append_50k", lambda: (tbl.append(make_batch(100_000, 50_000, city="LA", amt=2.5)), tbl.scan().count())[1])
snap2 = step("snapshot_after_append", lambda: tbl.current_snapshot().snapshot_id)

# time travel: read at snap1
step("time_travel_snap1_count", lambda: tbl.scan(snapshot_id=snap1).count())
step("time_travel_snap1_cities", lambda: sorted(set(tbl.scan(snapshot_id=snap1).to_arrow()["city"].to_pylist())))

# row-level delete via pyiceberg (equality delete path if supported)
step("row_delete_id_lt_1000", lambda: (tbl.delete(delete_filter=EqualTo("id", 5)), "deleted id=5 attempted"))
step("count_after_delete_attempt", lambda: tbl.scan().count())
step("id5_present", lambda: 5 in set(tbl.scan(row_filter=EqualTo("id", 5)).to_arrow()["id"].to_pylist()))

# update path: pyiceberg upsert if available
def try_upsert():
    upd = pa.table({"id": pa.array([10], type=pa.int64()), "city": pa.array(["SF"]), "amount": pa.array([9.9], type=pa.float64())}, schema=ARROW_SCHEMA)
    res = tbl.upsert(upd)
    row = tbl.scan(row_filter=EqualTo("id", 10)).to_arrow()
    return f"upsert matched={res.rows_matching} updated={res.rows_updated}; id10 city now {row['city'].to_pylist()}"
step("upsert_id10", try_upsert)

# small files: 50 tiny appends of 10 rows each on a second table
tbl2 = step("create_table_small", lambda: catalog.create_table("db.small", schema=schema, properties={"format-version": "2"}))
def small_appends():
    for i in range(50):
        tbl2.append(make_batch(i * 10, 10, city="X", amt=0.1))
    return tbl2.scan().count()
step("small_50_appends_rows", small_appends)

def count_data_files(t):
    n = 0
    for f in t.scan().plan_files():
        n += 1
    return n
step("small_table_data_files", lambda: count_data_files(tbl2))
step("main_table_data_files", lambda: count_data_files(tbl))
step("small_snapshots_count", lambda: len(tbl2.snapshots()))

# compaction API surface in pyiceberg?
step("pyiceberg_compaction_api", lambda: [a for a in dir(tbl) if "compact" in a.lower() or "rewrite" in a.lower()] or "none on Table")

with open(os.path.join(BASE, "py_results.txt"), "w") as f:
    f.write("\n".join(log) + "\n")
print("DONE")
