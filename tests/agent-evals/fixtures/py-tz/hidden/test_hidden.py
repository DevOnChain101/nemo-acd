import os
import sys
import time
import unittest
from datetime import timedelta

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.environ["TZ"] = "Asia/Tokyo"
if hasattr(time, "tzset"):
    time.tzset()

from tzparse import parse_ts, to_epoch  # noqa: E402


class HiddenTest(unittest.TestCase):
    def test_offset_is_kept(self):
        dt = parse_ts("2023-11-14T22:13:20+05:30")
        self.assertEqual(dt.utcoffset(), timedelta(hours=5, minutes=30))

    def test_offset_epoch(self):
        self.assertEqual(to_epoch("2023-11-14T22:13:20+05:30"), 1700000000 - 19800 + 0)

    def test_negative_offset(self):
        self.assertEqual(to_epoch("2023-11-14T17:13:20-05:00"), 1700000000)

    def test_naive_means_utc(self):
        self.assertEqual(to_epoch("2023-11-14T22:13:20"), 1700000000)

    def test_zulu_has_tzinfo(self):
        self.assertIsNotNone(parse_ts("2023-11-14T22:13:20Z").tzinfo)


if __name__ == "__main__":
    unittest.main()
