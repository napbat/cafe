package differential;

/** Deterministic object, array, exception, and static-state surfaces. */
public final class Things {
    public static int counter;
    private final int seed;

    public Things() {
        this.seed = 17;
    }

    private Things(int seed) {
        this.seed = seed;
    }

    public static int construct(int seed) {
        Things thing = new Things(seed);
        return thing.seed * 2 + new Things().seed;
    }

    public static int arraySum(int[] values) {
        if (values == null) {
            return -1;
        }
        int total = 0;
        for (int i = 0; i < values.length; i++) {
            total += values[i] * (i + 1);
        }
        return total;
    }

    public static int[] build(int size) {
        int[] values = new int[size];
        for (int i = 0; i < size; i++) {
            values[i] = i * i;
        }
        return values;
    }

    public static int matrix(int size) {
        int[][] cells = new int[size][size + 1];
        int total = 0;
        for (int r = 0; r < cells.length; r++) {
            for (int c = 0; c < cells[r].length; c++) {
                cells[r][c] = r * 10 + c;
                total += cells[r][c];
            }
        }
        return total;
    }

    public static int initialized() {
        int[] values = {5, 8, 13, 21};
        return values[2] + values.length;
    }

    public static String describe(Object value) {
        if (value instanceof String) {
            return "s:" + (String) value;
        }
        if (value instanceof Integer) {
            return "i:" + value;
        }
        return value == null ? "null" : "other";
    }

    public static int stringOps(String value) {
        if (value == null) {
            return -1;
        }
        return value.length() * 31 + (value.isEmpty() ? 7 : value.charAt(0));
    }

    public static int caught(int[] values, int index) {
        try {
            return values[index];
        } catch (ArrayIndexOutOfBoundsException error) {
            return -100;
        } catch (NullPointerException error) {
            return -200;
        }
    }

    public static int finallyCounts(int a) {
        int state = a;
        try {
            if (a > 0) {
                return state + counter;
            }
            state -= 5;
        } finally {
            counter += 1;
        }
        return state;
    }

    public static int nestedTry(String value) {
        try {
            try {
                return value.length();
            } catch (NullPointerException inner) {
                return 1;
            }
        } catch (RuntimeException outer) {
            return 3;
        }
    }

    public static int rethrown(int a) {
        try {
            return checked(a);
        } catch (Exception error) {
            return -1;
        }
    }

    private static int checked(int a) throws Exception {
        if (a < 0) {
            throw new Exception("negative");
        }
        return a * 3;
    }

    public static int stateMachine(int a) {
        counter = a;
        counter += bump();
        return counter;
    }

    private static int bump() {
        counter *= 2;
        return counter / 3;
    }

    public static int locked(int a) {
        synchronized (Things.class) {
            return a + 1;
        }
    }

    public static int lockedNested(int a) {
        synchronized (Things.class) {
            synchronized (String.class) {
                return a * 2 + 1;
            }
        }
    }

    public static int lockedThrow(int[] values) {
        synchronized (Things.class) {
            return values[0] + values.length;
        }
    }

    public static int chained(int a) {
        return new StringBuilder().append(a).append('x').length();
    }

    public static int finallyPlain(int a) {
        int result = 0;
        try {
            result = a * 2;
        } finally {
            counter += 2;
        }
        return result;
    }

    public static int finallyThrows(int a) {
        try {
            if (a < 0) {
                throw new IllegalStateException("negative");
            }
            return a + 1;
        } finally {
            counter += 3;
        }
    }

    public static int finallyBoth(int a, int b) {
        int total = 0;
        try {
            if (a > b) {
                return a - b;
            }
            total = a + b;
            return total * 2;
        } finally {
            counter += a;
            counter += 1;
        }
    }
}
