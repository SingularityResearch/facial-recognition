"""Copy one device's data/ and private folder into the shared database (dbimport.py does the work).

    python tools/db_import.py --data <data folder> --home <FaceLabels folder> [--device-id UUID] [--device-name NAME]
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import dbimport  # noqa: E402

if __name__ == "__main__":
    dbimport.main()
