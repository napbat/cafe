package legacy;

import java.util.function.Function;

public class LegacyCorpus<T> {
    private final T value;

    public LegacyCorpus(T value) {
        this.value = value;
    }

    public String render(int choice) {
        Function<T, String> function = item -> "value=" + item;
        switch (choice) {
            case 0:
                return function.apply(value);
            case 10:
                return "ten";
            default:
                return "other";
        }
    }
}
