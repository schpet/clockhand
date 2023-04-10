# clockhand

clockhand gives you a hand setting harvest timers.

## Commands

```bash
# shows a desktop notification if files are changed in a project and a timer
# isn't running
clockhand watch ~/code/*/clockhand.json

clockhand start
clockhand start --add 5 # adds 5 minutes
clockhand stop
clockhand status

# upserts that message into timer description, fails if timer isn't found
clockhand note "message..."
clockhand note --day 2023-04-12 "message..."

```
