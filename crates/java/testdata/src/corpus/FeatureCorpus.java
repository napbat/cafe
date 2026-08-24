package corpus;

import java.io.IOException;
import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;
import java.util.ArrayList;
import java.util.List;

@FeatureCorpus.RuntimeAnnotation(number = 42, type = String.class)
@FeatureCorpus.Nested("class")
public sealed class FeatureCorpus<T extends Number & Comparable<T>>
        permits FeatureCorpus.Child {
    public static final String CONSTANT = "constant";

    @Deprecated
    public @InvisibleTypeUse T value;

    public FeatureCorpus(T value) {
        this.value = value;
    }

    @RuntimeAnnotation(number = 7, type = Integer.class)
    public <U extends CharSequence> List<@TypeUse U> transform(
            @RuntimeAnnotation(number = 1, type = String.class)
            @Nested("parameter") final U input)
            throws IOException {
        class Local {
            String text() {
                return input.toString();
            }
        }

        Runnable lambda = () -> System.out.println(new Local().text());
        lambda.run();
        List<@TypeUse U> result = new ArrayList<>();
        if (input.length() == 0) {
            throw new IOException("empty");
        }
        result.add(input);
        return result;
    }

    public static final class Child extends FeatureCorpus<Integer> {
        public Child(Integer value) {
            super(value);
        }
    }

    public record Pair(@RuntimeAnnotation(number = 2, type = Long.class) int left,
                       @TypeUse String right) {}

    @Retention(RetentionPolicy.RUNTIME)
    @Target({ElementType.TYPE, ElementType.FIELD, ElementType.METHOD,
             ElementType.PARAMETER, ElementType.RECORD_COMPONENT})
    public @interface RuntimeAnnotation {
        int number() default 5;
        Class<?> type();
        Nested nested() default @Nested("nested");
        int[] values() default {1, 2, 3};
    }

    @Retention(RetentionPolicy.CLASS)
    public @interface Nested {
        String value();
    }

    @Retention(RetentionPolicy.RUNTIME)
    @Target(ElementType.TYPE_USE)
    public @interface TypeUse {}

    @Retention(RetentionPolicy.CLASS)
    @Target(ElementType.TYPE_USE)
    public @interface InvisibleTypeUse {}
}
