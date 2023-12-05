# AutoBib


## Lookup flow

Given a CLI call of the form
```
autobib <repo> <id>
```
perform the following pseudocode:
```
if local_cache:
  cached_records = deserialize(local_cache)
else:
  cached_records = initialize_cache()

if valid_repo(repo):
  if repo.validate_id(id):
    if (repo, id) in cached_records.keys():
      return cached_records[repo, id]
    else:
      record = repo.get_record(id)
      switch record
        if NetworkError:
          fail()
        else:
          cached_records[repo, id] = record
          return record
  else:
    return InvalidId
else:
  return InvalidRepository
```
