(ns jepsen.openraft.checker
  (:require [clojure.java.io :as io]
            [jepsen.checker :as checker]
            [jepsen.store :as store]))

;; Emitted by openraft-test-app's panic hook.
(def node-panic-pattern
  #"OPENRAFT_JEPSEN_PANIC")

(defn random-seed-checker []
  (reify checker/Checker
    (check [_ test _history _opts]
      {:valid? true
       :seed (:seed test)})))

(defn strict-unhandled-exceptions []
  (let [delegate (checker/unhandled-exceptions)]
    (reify checker/Checker
      (check [_ test history opts]
        (let [result (checker/check delegate test history opts)]
          (if (seq (:exceptions result))
            (assoc result :valid? :unknown)
            result))))))

(defn- log-matches [pattern node file]
  (with-open [reader (io/reader file)]
    (into []
          (comp (filter #(re-find pattern %))
                (map (fn [line]
                       {:node node
                        :line line})))
          (line-seq reader))))

(defn required-log-file-pattern [pattern filename]
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
         :matches matches}))))
