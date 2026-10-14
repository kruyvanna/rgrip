-- Demo data for screenshots: Phnom Penh parcels, owners and inspections.
-- psql "host=localhost port=55432 user=rgrip dbname=rgrip" -f dev/demo.sql
drop schema if exists demo cascade;
create schema demo;
select setseed(0.42);

create table demo.districts (
  id serial primary key,
  name text not null,
  geom geometry(Polygon, 4326)
);
insert into demo.districts (name, geom) values
  ('Doun Penh', st_makeenvelope(104.910, 11.560, 104.935, 11.590, 4326)),
  ('Chamkar Mon', st_makeenvelope(104.910, 11.530, 104.935, 11.560, 4326)),
  ('7 Makara', st_makeenvelope(104.895, 11.555, 104.910, 11.580, 4326)),
  ('Toul Kork', st_makeenvelope(104.880, 11.565, 104.905, 11.600, 4326)),
  ('Boeng Keng Kang', st_makeenvelope(104.900, 11.530, 104.915, 11.555, 4326)),
  ('Chbar Ampov', st_makeenvelope(104.935, 11.505, 104.965, 11.550, 4326));

create table demo.owners (
  id serial primary key,
  full_name text not null,
  phone text,
  registered_on date,
  verified boolean default false,
  updated_at timestamptz default now()
);
insert into demo.owners (full_name, phone, registered_on, verified, updated_at)
select n.full_name,
       '+855 ' || (10 + (random() * 89)::int) || ' ' || lpad((random() * 999999)::int::text, 6, '0'),
       date '2018-01-01' + (random() * 2500)::int,
       random() > 0.3,
       timestamptz '2026-09-01 08:00+07' + (random() * 40 * 86400) * interval '1 second'
from (values
  ('Sok Dara'), ('Chan Sophea'), ('Keo Vibol'),
  ('Lim Srey Pich'), ('Heng Bunthoeun'), ('Meas Sokha'),
  ('Ouk Rithy'), ('Phan Sreymom'), ('Ly Chanthou'),
  ('Touch Vanna'), ('Nhem Kosal'), ('Yim Sopheak')
) as n(full_name);

create table demo.parcels (
  id serial primary key,
  parcel_code text unique not null,
  owner_id int references demo.owners,
  district_id int references demo.districts,
  land_use text,
  area_m2 numeric(10, 1),
  price_usd numeric(12, 0),
  surveyed_at timestamp,
  note text,
  geom geometry(Polygon, 4326)
);
insert into demo.parcels (parcel_code, owner_id, district_id, land_use, area_m2, price_usd, surveyed_at, note, geom)
select 'PP-' || d.id || '-' || lpad(g::text, 4, '0'),
       1 + (random() * 11)::int,
       d.id,
       (array['residential', 'commercial', 'mixed', 'garden', 'public'])[1 + (random() * 4)::int],
       0, (40000 + random() * 900000)::int,
       timestamp '2025-01-01' + (random() * 600 * 86400) * interval '1 second',
       (array['Shophouse', 'Vacant lot', 'Office block', 'Garden', 'Street frontage'])[1 + (random() * 4)::int],
       st_rotate(
         st_makeenvelope(x, y, x + 0.0004 + random() * 0.0006, y + 0.0003 + random() * 0.0005, 4326),
         random() * 0.5, st_point(x, y))
from demo.districts d,
     generate_series(1, 18) g,
     lateral (select st_xmin(d.geom) + 0.002 + random() * (st_xmax(d.geom) - st_xmin(d.geom) - 0.005) as x,
                     st_ymin(d.geom) + 0.002 + random() * (st_ymax(d.geom) - st_ymin(d.geom) - 0.005) as y) p;
update demo.parcels set area_m2 = round(st_area(geom::geography)::numeric, 1);

create table demo.inspections (
  id serial primary key,
  parcel_id int not null references demo.parcels,
  inspector text,
  inspected_on date,
  status text,
  notes text
);
insert into demo.inspections (parcel_id, inspector, inspected_on, status, notes)
select 1 + (random() * 107)::int,
       (array['Dara', 'Sophea', 'Vibol', 'Kosal'])[1 + (random() * 3)::int],
       date '2026-01-05' + (random() * 270)::int,
       (array['approved', 'pending', 'needs fix'])[1 + (random() * 2)::int],
       (array['Boundary matches survey', 'Fence on neighbour land', 'Title deed verified', 'Boundary marker replaced', null])[1 + (random() * 4)::int]
from generate_series(1, 160);

create table demo.places (
  id serial primary key,
  name text, category text, opened_on date,
  geom geometry(Point, 4326)
);
insert into demo.places (name, category, opened_on, geom) values
  ('Royal Palace', 'landmark', '1866-01-01', st_point(104.9310, 11.5640, 4326)),
  ('Wat Phnom', 'temple', '1372-01-01', st_point(104.9230, 11.5760, 4326)),
  ('Central Market', 'market', '1937-01-01', st_point(104.9205, 11.5695, 4326)),
  ('Independence Monument', 'landmark', '1958-11-09', st_point(104.9282, 11.5563, 4326)),
  ('Russian Market', 'market', '1980-01-01', st_point(104.9180, 11.5440, 4326)),
  ('Olympic Stadium', 'sport', '1964-01-01', st_point(104.9110, 11.5590, 4326)),
  ('National Museum', 'museum', '1920-01-01', st_point(104.9295, 11.5660, 4326)),
  ('Toul Sleng', 'museum', '1980-01-01', st_point(104.9175, 11.5495, 4326));

-- A legacy schema with primary keys but no foreign keys: rgrip infers the
-- relations from column names (orders.customer_id → customers.id, …).
drop schema if exists legacy cascade;
create schema legacy;
create table legacy.customers (id int primary key, name text, city text, joined_on date);
insert into legacy.customers
select g, (array['Sok Dara', 'Chan Sophea', 'Keo Vibol', 'Lim Srey Pich', 'Heng Bunthoeun', 'Meas Sokha'])[g],
       (array['Phnom Penh', 'Siem Reap', 'Battambang', 'Kampot', 'Phnom Penh', 'Takeo'])[g],
       date '2021-01-01' + (g * 97)
from generate_series(1, 6) g;
create table legacy.orders (id int primary key, customer_id int, total_usd numeric(10, 2), status text, ordered_at timestamptz);
insert into legacy.orders
select g, 1 + (g % 6), round((15 + random() * 400)::numeric, 2),
       (array['paid', 'shipped', 'pending'])[1 + (g % 3)],
       timestamptz '2026-09-01 09:00+07' + g * interval '7 hours'
from generate_series(1, 40) g;
create table legacy.order_items (id int primary key, order_id int, product text, qty int);
insert into legacy.order_items
select g, 1 + (g % 40), (array['Coffee beans', 'Rice 25kg', 'Palm sugar', 'Kampot pepper', 'Fish sauce'])[1 + (g % 5)], 1 + (g % 4)
from generate_series(1, 120) g;
create table legacy.payments (id int primary key, order_id int, amount_usd numeric(10, 2), paid_on date);
insert into legacy.payments
select g, g, round((15 + random() * 400)::numeric, 2), date '2026-09-02' + g
from generate_series(1, 30) g;
