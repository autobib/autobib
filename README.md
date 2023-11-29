# AutoBib


## Lookup flow

Given a CLI call of the form
```
autobib <source> <id>
```
perform the following pseudocode:
```
if local_cache:
  cached_records = deserialize(local_cache)
else:
  cached_records = initialize_cache()

if valid_source(source):
  if source.validate_id(id):
    if (source, id) in cached_records.keys():
      return cached_records[source, id]
    else:
      record = source.get_record(id)
      switch record
        if NetworkError:
          fail()
        else:
          cached_records[source, id] = record
          return record
  else:
    return InvalidId
else:
  return InvalidSource
```
