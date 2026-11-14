"""Unit tests for the physical-line counting function."""

import unittest

import fixtures
from file_limits.counting import count_physical_lines


def lines(count, final_newline=True):
    return fixtures.n_lines(count, final_newline=final_newline)


class CountPhysicalLinesTests(unittest.TestCase):
    def test_empty_file_counts_zero(self):
        self.assertEqual(count_physical_lines(b""), 0)

    def test_one_line(self):
        self.assertEqual(count_physical_lines(lines(1)), 1)

    def test_249_lines(self):
        self.assertEqual(count_physical_lines(lines(249)), 249)

    def test_250_lines_is_exactly_the_limit(self):
        self.assertEqual(count_physical_lines(lines(250)), 250)

    def test_251_lines_exceed_the_limit(self):
        self.assertEqual(count_physical_lines(lines(251)), 251)

    def test_250_lines_without_final_newline_still_count_250(self):
        self.assertEqual(count_physical_lines(lines(250, final_newline=False)), 250)

    def test_251_lines_without_final_newline_still_count_251(self):
        self.assertEqual(count_physical_lines(lines(251, final_newline=False)), 251)

    def test_missing_final_newline_counts_the_partial_line(self):
        self.assertEqual(count_physical_lines(b"a\nb"), 2)

    def test_crlf_counts_one_line_per_pair(self):
        self.assertEqual(count_physical_lines(b"a\r\nb\r\n"), 2)

    def test_crlf_250_lines(self):
        self.assertEqual(count_physical_lines(b"row\r\n" * 250), 250)

    def test_crlf_251_lines(self):
        self.assertEqual(count_physical_lines(b"row\r\n" * 251), 251)

    def test_blank_only_lines_count(self):
        self.assertEqual(count_physical_lines(b"\n\n\n\n"), 4)

    def test_comment_only_lines_count(self):
        self.assertEqual(count_physical_lines(b"// one\n// two\n"), 2)
        self.assertEqual(count_physical_lines(b"# one\n# two\n"), 2)

    def test_doc_comment_lines_count(self):
        self.assertEqual(count_physical_lines(b"/// doc\n//! inner\n"), 2)
        self.assertEqual(count_physical_lines(b'"""docs"""\n'), 1)


if __name__ == "__main__":
    unittest.main()