UPDATE accounts SET color = 'account-' || substr(color, 7) WHERE color LIKE 'chart-%';
