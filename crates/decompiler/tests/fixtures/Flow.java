package differential;

/** Deterministic control-flow surfaces: loops, switches, conditions. */
public final class Flow {
    private Flow() {}

    public static int sumTo(int n) {
        int total = 0;
        for (int i = 0; i < n; i++) {
            total += i;
        }
        return total;
    }

    public static int collatzSteps(int n) {
        int steps = 0;
        while (n > 1) {
            n = (n % 2 == 0) ? n / 2 : 3 * n + 1;
            steps++;
            if (steps > 40) {
                break;
            }
        }
        return steps;
    }

    public static int doubleUp(int n) {
        int value = n;
        do {
            value = value * 2 + 1;
        } while (value < 40 && value > -40);
        return value;
    }

    public static int nested(int rows, int columns) {
        int cells = 0;
        for (int r = 0; r < rows; r++) {
            for (int c = 0; c < columns; c++) {
                if ((r + c) % 3 == 0) {
                    continue;
                }
                cells += r * c + 1;
            }
        }
        return cells;
    }

    public static int labeled(int limit) {
        int found = -1;
        outer:
        for (int a = 0; a < limit; a++) {
            for (int b = 0; b < limit; b++) {
                if (a * b == 12) {
                    found = a * 100 + b;
                    break outer;
                }
            }
        }
        return found;
    }

    public static int table(int key) {
        switch (key) {
            case 0:
                return 10;
            case 1:
                return 11;
            case 2:
                return 12;
            case 3:
                return 13;
            default:
                return -1;
        }
    }

    public static int lookup(int key) {
        switch (key) {
            case -3:
                return 1;
            case 7:
                return 2;
            case 4096:
                return 3;
            default:
                return 0;
        }
    }

    public static int fallthrough(int key) {
        int total = 0;
        switch (key) {
            case 0:
                total += 1;
            case 1:
                total += 2;
                break;
            case 2:
                total += 4;
            default:
                total += 8;
        }
        return total;
    }

    public static int stringSwitch(String value) {
        if (value == null) {
            return -2;
        }
        switch (value) {
            case "":
                return 0;
            case "cafe":
                return 1;
            case "Ea":
                return 2;
            case "FB":
                return 3;
            default:
                return -1;
        }
    }

    public static boolean shortCircuit(String value, int limit) {
        return value != null && value.length() > limit;
    }

    public static int ternaryChain(int a) {
        return a < 0 ? -1 : (a == 0 ? 0 : (a > 100 ? 2 : 1));
    }

    public static int earlyReturns(int a, int b) {
        if (a > b) {
            return a - b;
        }
        if (a == b) {
            return 0;
        }
        return b - a;
    }

    public static int booleanOps(boolean a, boolean b) {
        int total = 0;
        if (a || b) {
            total += 1;
        }
        if (a && !b) {
            total += 2;
        }
        if (a ^ b) {
            total += 4;
        }
        return total;
    }

    public static int oddFactorial(int n) {
        if (n <= 1) {
            return 1;
        }
        return n * oddFactorial(n - 2);
    }
}
