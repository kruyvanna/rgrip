-- Sample spatial data around Phnom Penh for map testing.
create extension if not exists postgis;
create table places (id serial primary key, name text, kind text, geom geometry(Point, 4326), name_km text);
insert into places (name, kind, geom, name_km) values
  ('Royal Palace', 'landmark', st_setsrid(st_makepoint(104.9311, 11.5637), 4326), 'ព្រះបរមរាជវាំង'),
  ('Central Market', 'market', st_setsrid(st_makepoint(104.9213, 11.5694), 4326), 'ផ្សារធំថ្មី'),
  ('Wat Phnom', 'temple', st_setsrid(st_makepoint(104.9230, 11.5760), 4326), 'វត្តភ្នំ'),
  ('Independence Monument', 'landmark', st_setsrid(st_makepoint(104.9282, 11.5564), 4326), 'វិមានឯករាជ្យ'),
  ('Russian Market', 'market', st_setsrid(st_makepoint(104.9182, 11.5397), 4326), 'ផ្សារទួលទំពូង');
create table roads (id serial primary key, name text, geom geometry(LineString, 4326));
insert into roads (name, geom) values
  ('Sisowath Quay', st_geomfromtext('LINESTRING(104.9318 11.5600, 104.9320 11.5660, 104.9290 11.5730)', 4326)),
  ('Norodom Blvd', st_geomfromtext('LINESTRING(104.9282 11.5450, 104.9282 11.5564, 104.9255 11.5700)', 4326));
create table zones (id serial primary key, name text, geom geometry(Polygon, 4326));
insert into zones (name, geom) values
  ('Daun Penh core', st_geomfromtext('POLYGON((104.918 11.560, 104.934 11.560, 104.934 11.580, 104.918 11.580, 104.918 11.560))', 4326));
create table parcels_utm (id serial primary key, geom geometry(Polygon, 32648));
insert into parcels_utm (geom) select st_transform(st_buffer(geom::geography, 150)::geometry, 32648) from places limit 2;
