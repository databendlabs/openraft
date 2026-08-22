(ns jepsen.openraft.checker
  (:require [clojure.java.io :as io]
            [jepsen.checker :as checker]
            [jepsen.openraft.harness :as harness]
            [jepsen.openraft.interruption :as interruption]
            [jepsen.store :as store]))

;; Emitted by openraft-test-app's panic hook.
(def node-panic-pattern
  #"OPENRAFT_JEPSEN_PANIC")

(defn- checker-exception-evidence [throwable]
  {:class (.getName (class throwable))
   :message (ex-message throwable)})

(defn reject-checker-exceptions
  "Turns non-interruption checker exceptions into Harness evidence.

  The wrapper is suitable for both standalone and composed checkers."
  [delegate]
  (reify
    checker/Checker
    (check [_ test history opts]
      (try
        (checker/check delegate test history opts)
        (catch Throwable throwable
          (cond
            (interruption/interruption? throwable)
            (do
              (.interrupt (Thread/currentThread))
              (throw throwable))

            (interruption/fatal-throwable? throwable)
            (throw throwable)

            :else
            {:valid? false
             :checker-exception (checker-exception-evidence throwable)}))))))

(defn random-seed-checker []
  (reject-checker-exceptions
   (reify checker/Checker
     (check [_ test _history _opts]
       {:valid? true
        :seed (:seed test)}))))

(defn- escaped-worker-evidence [exceptions]
  (mapv (fn [exception]
          (update exception :example #(when % (into {} %))))
        exceptions))

(defn strict-unhandled-exceptions []
  (let [delegate (checker/unhandled-exceptions)]
    (reject-checker-exceptions
     (reify checker/Checker
       (check [_ test history opts]
         (let [result (checker/check delegate test history opts)]
           (if (seq (:exceptions result))
             (assoc result
                    :valid? false
                    :escaped-worker-exceptions
                    (escaped-worker-evidence (:exceptions result)))
             result)))))))

(defn- failure-evidence [{:keys [source context throwable]}]
  {:source source
   :context context
   :exception {:class (.getName (class throwable))
               :message (ex-message throwable)}})

(defrecord HarnessFailureChecker [failure-state delegate strict-checker-key]
  checker/Checker
  (check [_ test history opts]
    (let [{:keys [primary secondary]}
          (harness/failure-snapshot failure-state)
          result (checker/check delegate test history opts)
          strict-evidence
          (when strict-checker-key
            (get-in result
                    [strict-checker-key :escaped-worker-exceptions]))]
      (if (or primary (seq strict-evidence))
        (assoc result
               :valid? false
               :harness-failure
               (cond-> {:valid? false}
                 primary
                 (assoc :primary (failure-evidence primary)
                        :secondary (mapv failure-evidence secondary))

                 (seq strict-evidence)
                 (assoc :strict-fallback
                        {:escaped-worker-exceptions strict-evidence})))
        result))))

(defn reject-harness-failures
  "Rejects recorded Harness failures after retaining aggregate analysis.

  When strict-checker-key is supplied, escaped-worker evidence from that
  composed child also rejects the final verdict as a post-hoc fallback."
  ([failure-state delegate]
   (HarnessFailureChecker. failure-state delegate nil))
  ([failure-state delegate strict-checker-key]
   (HarnessFailureChecker. failure-state delegate strict-checker-key)))

(defn- log-matches [pattern node file]
  (with-open [reader (io/reader file)]
    (into []
          (comp (filter #(re-find pattern %))
                (map (fn [line]
                       {:node node
                        :line line})))
          (line-seq reader))))

(defn required-log-file-pattern [pattern filename]
  (reject-checker-exceptions
   (reify checker/Checker
     (check [_ test _history _opts]
       (let [node-files (mapv (fn [node]
                                [node (store/path test node filename)])
                              (:nodes test))
             missing-nodes (->> node-files
                                (remove (fn [[_ file]]
                                          (.isFile ^java.io.File file)))
                                (mapv first))
             matches (->> node-files
                          (filter (fn [[_ file]]
                                    (.isFile ^java.io.File file)))
                          (mapcat (fn [[node file]]
                                    (log-matches pattern node file)))
                          vec)
             valid? (cond
                      (seq matches) false
                      (seq missing-nodes) :unknown
                      :else true)]
         {:valid? valid?
          :filename filename
          :missing-nodes missing-nodes
          :count (count matches)
          :matches matches})))))
