# clockhand

clockhand gives you a hand setting harvest timers.

## Commands

```bash
clockhand start
clockhand start --add 5 # adds 5 minutes
clockhand stop

# upserts that message into timer description, fails if timer isn't found
clockhand note "message..."
clockhand note --day 2023-04-12 "message..."

clockhand watch ~/code/**/clockhand.json
```
