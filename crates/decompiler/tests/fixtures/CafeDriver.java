import java.lang.reflect.Array;
import java.lang.reflect.InvocationTargetException;
import java.lang.reflect.Method;
import java.lang.reflect.Modifier;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Comparator;
import java.util.List;
import java.util.TreeSet;

/**
 * Differential execution driver: enumerates the target class's public
 * static methods in deterministic order, invokes each with deterministic
 * argument sets, and prints one line per invocation — the result value, or
 * the thrown exception's class (never its message, which may lawfully
 * differ between compilations). The transcript is the behavioral
 * fingerprint compared between the original and decompiled classes.
 */
public final class CafeDriver {
    private CafeDriver() {}

    public static void main(String[] args) throws Exception {
        Class<?> target = Class.forName(args[0]);
        TreeSet<String> skip = new TreeSet<String>();
        for (int i = 1; i < args.length; i++) {
            skip.add(args[i]);
        }
        Method[] methods = target.getDeclaredMethods();
        Arrays.sort(methods, new Comparator<Method>() {
            public int compare(Method a, Method b) {
                return identity(a).compareTo(identity(b));
            }
        });
        StringBuilder out = new StringBuilder();
        for (Method method : methods) {
            int modifiers = method.getModifiers();
            if (!Modifier.isStatic(modifiers) || !Modifier.isPublic(modifiers)
                    || method.isSynthetic()) {
                continue;
            }
            String id = identity(method);
            if (skip.contains(id)) {
                out.append(id).append(" skipped\n");
                continue;
            }
            List<Object[]> sets = argumentSets(method.getParameterTypes());
            if (sets == null) {
                out.append(id).append(" unsampled\n");
                continue;
            }
            for (Object[] arguments : sets) {
                out.append(id).append(' ').append(render(arguments)).append(" -> ");
                try {
                    out.append(renderValue(method.invoke(null, arguments)));
                } catch (InvocationTargetException error) {
                    out.append("throws ").append(error.getCause().getClass().getName());
                }
                out.append('\n');
            }
        }
        System.out.print(out);
    }

    static String identity(Method method) {
        StringBuilder text = new StringBuilder(method.getName());
        text.append('(');
        for (Class<?> parameter : method.getParameterTypes()) {
            text.append(descriptor(parameter));
        }
        return text.append(')').append(descriptor(method.getReturnType())).toString();
    }

    static String descriptor(Class<?> type) {
        if (type == void.class) {
            return "V";
        }
        if (type == int.class) {
            return "I";
        }
        if (type == long.class) {
            return "J";
        }
        if (type == float.class) {
            return "F";
        }
        if (type == double.class) {
            return "D";
        }
        if (type == boolean.class) {
            return "Z";
        }
        if (type == byte.class) {
            return "B";
        }
        if (type == char.class) {
            return "C";
        }
        if (type == short.class) {
            return "S";
        }
        if (type.isArray()) {
            return type.getName().replace('.', '/');
        }
        return "L" + type.getName().replace('.', '/') + ";";
    }

    static Object[] candidates(Class<?> type) {
        if (type == int.class) {
            return new Object[] {0, 1, -3, 7, Integer.MIN_VALUE};
        }
        if (type == long.class) {
            return new Object[] {0L, 1L, -3L, Long.MIN_VALUE};
        }
        if (type == float.class) {
            return new Object[] {0.0f, -0.0f, 1.5f, Float.NaN};
        }
        if (type == double.class) {
            return new Object[] {0.0d, -0.0d, 2.5d, Double.NaN};
        }
        if (type == boolean.class) {
            return new Object[] {Boolean.FALSE, Boolean.TRUE};
        }
        if (type == char.class) {
            return new Object[] {'a', (char) 0};
        }
        if (type == byte.class) {
            return new Object[] {(byte) -2, (byte) 5};
        }
        if (type == short.class) {
            return new Object[] {(short) -2, (short) 300};
        }
        if (type == String.class) {
            return new Object[] {null, "", "cafe", "Ea"};
        }
        if (type == int[].class) {
            return new Object[] {null, new int[0], new int[] {3, 1, 2}};
        }
        if (type == Object.class) {
            return new Object[] {null, "cafe", Integer.valueOf(9)};
        }
        return null;
    }

    /** Deterministic capped cartesian product; null for unsupported types. */
    static List<Object[]> argumentSets(Class<?>[] parameters) {
        long total = 1;
        for (Class<?> parameter : parameters) {
            Object[] values = candidates(parameter);
            if (values == null) {
                return null;
            }
            total *= values.length;
        }
        int limit = (int) Math.min(total, 32L);
        List<Object[]> sets = new ArrayList<Object[]>();
        for (int index = 0; index < limit; index++) {
            Object[] arguments = new Object[parameters.length];
            int remaining = index;
            for (int position = 0; position < parameters.length; position++) {
                Object[] values = candidates(parameters[position]);
                arguments[position] = values[remaining % values.length];
                remaining /= values.length;
            }
            sets.add(arguments);
        }
        return sets;
    }

    static String render(Object[] arguments) {
        StringBuilder text = new StringBuilder("(");
        for (int i = 0; i < arguments.length; i++) {
            if (i > 0) {
                text.append(", ");
            }
            text.append(renderValue(arguments[i]));
        }
        return text.append(')').toString();
    }

    static String renderValue(Object value) {
        if (value == null) {
            return "null";
        }
        if (value.getClass().isArray()) {
            int length = Array.getLength(value);
            StringBuilder text = new StringBuilder("[");
            for (int i = 0; i < length; i++) {
                if (i > 0) {
                    text.append(", ");
                }
                text.append(renderValue(Array.get(value, i)));
            }
            return text.append(']').toString();
        }
        if (value instanceof String) {
            return "\"" + value + "\"";
        }
        if (value instanceof Boolean || value instanceof Character || value instanceof Number) {
            return String.valueOf(value);
        }
        // Identity-based toString is nondeterministic; the class suffices.
        return value.getClass().getName();
    }
}
