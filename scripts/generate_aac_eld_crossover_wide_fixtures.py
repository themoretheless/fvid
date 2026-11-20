"""Own factor-two/four LF-bank crossover movies, independent/offline."""
from generate_aac_eld_crossover_fixtures import generate as crossover,SOURCE
OUT=SOURCE/'aac-eld-crossover-wide'
def generate():return crossover((2,4))
if __name__=='__main__':
    OUT.mkdir(exist_ok=True)
    for name,data in generate().items():(OUT/name).write_bytes(data)
