-- V2 : data lineage au niveau des tables ET des colonnes.
CREATE VIEW analytics.client_revenue AS
SELECT c.id AS customer_id, SUM(o.amount) AS total_revenue
FROM crm.customers c
JOIN sales.orders o ON c.id = o.customer_id
GROUP BY c.id;

CREATE VIEW reporting.dashboard AS
SELECT a.customer_id, a.total_revenue
FROM analytics.client_revenue a;

-- Sous-requete derivee, projections non ambigues.
CREATE VIEW reporting.expensive_orders AS
SELECT d.order_id, d.amount
FROM (
  SELECT o.id AS order_id, o.amount
  FROM sales.orders o
  WHERE o.amount > 100
) d;
