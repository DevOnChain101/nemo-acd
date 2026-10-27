import os
import sys
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
os.environ["TZ"] = "America/New_York"
if hasattr(time, "tzset"):
    time.tzset()

from tzparse import to_epoch  # noqa: E402


class ParseTest(unittest.TestCase):
    def test_zulu(self):
        self.assertEqual(to_epoch("2023-11-14T22:13:20Z"), 1700000000)

    def test_epoch_zero(self):
        self.assertEqual(to_epoch("1970-01-01T00:00:00+00:00"), 0)


if __name__ == "__main__":
    unittest.main()
