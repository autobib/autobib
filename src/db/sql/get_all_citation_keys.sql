SELECT name FROM CitationKeys
INNER JOIN Records ON CitationKeys.record_key = Records.key
WHERE Records.active = 1
