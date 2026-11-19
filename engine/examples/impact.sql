CREATE VIEW analytics.clients_actifs AS
SELECT c.customer_id, COUNT(o.order_id) AS total_commandes,
       SUM(o.amount) AS chiffre_affaires
FROM crm.customers c
JOIN sales.orders o ON c.customer_id = o.customer_id
GROUP BY c.customer_id;

CREATE VIEW reporting.dashboard AS
SELECT ca.customer_id, ca.chiffre_affaires
FROM analytics.clients_actifs ca;
