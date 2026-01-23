# Quick Task 010: Remove docker-compose from Docker image

## Objective

Remove the docker-cli-compose package from the runtime Docker image since docker-compose is no longer used after quick task 009.

## Tasks

### Task 1: Remove docker-cli-compose from Dockerfile

**File:** `Dockerfile`

Remove `docker-cli-compose` from the apk add command in the runtime stage.

**Before:**
```dockerfile
RUN apk add --no-cache git docker-cli docker-cli-compose
```

**After:**
```dockerfile
RUN apk add --no-cache git docker-cli
```

## Impact

- Smaller Docker image (docker-cli-compose adds ~15MB)
- Cleaner dependency tree
- No functionality lost (docker compose exec replaced with docker exec in task 009)
