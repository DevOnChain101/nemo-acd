from datetime import datetime, timezone


def parse_ts(s):
    """Parse an ISO-8601 timestamp. A missing offset means UTC."""
    dt = datetime.fromisoformat(s.replace("Z", "+00:00"))
    return dt if dt.tzinfo else dt.replace(tzinfo=timezone.utc)


def to_epoch(s):
    return parse_ts(s).timestamp()
