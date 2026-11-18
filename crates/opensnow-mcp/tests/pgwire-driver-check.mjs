import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { Client } = require(process.env.OPENSNOW_PG_MODULE || 'pg');
const connectionString = process.env.OPENSNOW_PG_URL;
assert.ok(connectionString, 'Set OPENSNOW_PG_URL to an opensnow PostgreSQL connection string');

const client = new Client({ connectionString });
await client.connect();
try {
  await client.query("SET TIME ZONE 'UTC'");
  console.log('PASS stock pg login and UTC session setup');

  const tables = await client.query(
    'SELECT table_schema, table_name FROM information_schema.tables ORDER BY table_schema, table_name',
  );
  assert.ok(tables.rows.some(({ table_name }) => table_name === 'facts'), 'Expected the facts fixture');
  console.log('PASS stock pg table discovery');

  const result = await client.query({
    text: `WITH "__opensight_filtered" AS (SELECT * FROM "facts" WHERE "dept" = $1)
      SELECT "amount" AS "amount" FROM "__opensight_filtered" ORDER BY "amount"`,
    values: ['1'],
    rowMode: 'array',
    types: { getTypeParser: () => (value) => value },
  });
  assert.deepEqual(result.rows, [['10'], ['30']]);
  assert.equal(result.rowCount, 2);
  console.log('PASS stock pg parameterized query and exact result rows');
} finally {
  await client.end();
}
