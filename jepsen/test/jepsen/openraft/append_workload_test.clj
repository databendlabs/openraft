(ns jepsen.openraft.append-workload-test
  (:require [clojure.java.io :as io]
            [clojure.test :refer [deftest is testing]]
            [jepsen.checker :as checker]
            [jepsen.client :as client]
            [jepsen.generator :as gen]
            [jepsen.generator.test :as gen-test]
            [jepsen.history :as history]
            [jepsen.openraft.append-workload :as append-workload]
            [jepsen.openraft.client :as http]
            [jepsen.store :as store]
            [rhizome.viz :as rhizome]))

(defn- append-client [appended-keys]
  (append-workload/->AppendClient (atom "n1:21001")
                                  ["n1:21001"]
                                  appended-keys))

(defn- txn [type micro-op & {:as extra}]
  (merge {:type type :process 0 :f :txn :value [micro-op] :phase :main}
         extra))

(deftest maps-micro-operations-to-list-requests
  (let [appended-keys (atom (sorted-set))
        subject (append-client appended-keys)
        calls (atom [])]
    (with-redefs [http/append! (fn [endpoint k v]
                                 (swap! calls conj [:append endpoint k v])
                                 {:value v :version 7})
                  http/linearizable-list-read! (fn [endpoint k]
                                                 (swap! calls conj [:read endpoint k])
                                                 ["1" "2"])]
      (is (= :ok (:type (client/invoke! subject {} (txn :invoke [:append 3 2])))))
      (is (= [[:r 3 [1 2]]]
             (:value (client/invoke! subject {} (txn :invoke [:r 3 nil])))))
      (is (= [[:append "n1:21001" "jepsen-list-3" "2"]
              [:read "n1:21001" "jepsen-list-3"]]
             @calls))
      (is (= #{3} @appended-keys)))))

(deftest classifies-failures-by-whether-the-append-may-have-applied
  (let [subject (append-client (atom (sorted-set)))
        timeout (ex-info "timeout" {:kind :request-timeout})
        discarded (ex-info "discarded"
                           {:kind :openraft-error
                            :error {:LogEntryDiscarded
                                    {:leader_id "n2"
                                     :leader_node {:data "n2:21001"}}}})]
    (with-redefs [http/append! (fn [& _] (throw timeout))
                  http/linearizable-list-read! (fn [& _] (throw timeout))]
      (is (= :info (:type (client/invoke! subject {} (txn :invoke [:append 1 1])))))
      (is (= :fail (:type (client/invoke! subject {} (txn :invoke [:r 1 nil]))))))
    (with-redefs [http/append! (fn [& _] (throw discarded))]
      (let [result (client/invoke! subject {} (txn :invoke [:append 1 1]))]
        (is (= :info (:type result)))
        (is (= :log-entry-discarded (:error result)))
        (is (not (contains? result :unexpected-sut-response?)))))
    (testing "a multi-operation transaction is a Harness failure"
      (is (thrown? clojure.lang.ExceptionInfo
                   (client/invoke! subject {}
                                   (assoc (txn :invoke [:r 1 nil])
                                          :value [[:r 1 nil] [:append 1 2]])))))))

(deftest generates-single-operation-transactions
  (let [ops (->> (gen-test/simulate
                  (gen/limit 50 (:generator (append-workload/workload {})))
                  (fn [_test op] (assoc op :type :ok)))
                 (filter #(= :invoke (:type %))))]
    (is (= 50 (count ops)))
    (is (every? #(and (= :txn (:f %))
                      (= :main (:phase %))
                      (= 1 (count (:value %)))
                      (#{:append :r} (ffirst (:value %))))
                ops))
    (is (some #(= :append (ffirst (:value %))) ops))
    (is (some #(= :r (ffirst (:value %))) ops))))

(deftest final-recovery-reads-every-appended-list
  (let [wl (append-workload/workload {})
        subject (client/open! (:client wl) {:nodes ["n1"]} "n1")]
    (with-redefs [http/append! (fn [_ _ v] {:value v :version 1})]
      (doseq [k [4 2]]
        (client/invoke! subject {} (txn :invoke [:append k 1]))))
    (is (= [[[[:r 2 nil]] :final true]
            [[[:r 4 nil]] :final true]]
           (->> (gen-test/simulate (:final-generator wl)
                                   (fn [_test op] (assoc op :type :ok)))
                (filter #(= :invoke (:type %)))
                (map (juxt :value :phase :final?)))))))

(defn- check-in-temp-store [ops]
  (let [^java.io.File temp-dir
        (.toFile
         (java.nio.file.Files/createTempDirectory
          "openraft-append-checker"
          (make-array java.nio.file.attribute.FileAttribute 0)))
        test {:name "append-checker-test"
              :start-time "run"}]
    (try
      ;; Elle plots each cycle with GraphViz, which unit tests do not need.
      (with-redefs [store/base-dir (.getPath temp-dir)
                    rhizome/dot->svg (constantly "<svg/>")]
        (checker/check (:checker (append-workload/workload {}))
                       test
                       (history/history ops)
                       {}))
      (finally
        (doseq [file (reverse (file-seq temp-dir))]
          (io/delete-file file true))))))

(defn- ops [& pairs]
  (mapcat (fn [[invoke complete]]
            [(txn :invoke invoke) (txn :ok complete)])
          pairs))

(deftest elle-sees-an-element-applied-twice
  (testing "appends read back once, in order, pass"
    (let [result (check-in-temp-store
                  (ops [[:append 1 1] [:append 1 1]]
                       [[:append 1 2] [:append 1 2]]
                       [[:r 1 nil] [:r 1 [1 2]]]))]
      (is (true? (:valid? result)))
      (is (true? (get-in result [:elle :valid?])))))

  (testing "an element read twice fails"
    (let [result (check-in-temp-store
                  (ops [[:append 1 1] [:append 1 1]]
                       [[:append 1 2] [:append 1 2]]
                       [[:r 1 nil] [:r 1 [1 2 1]]]))]
      (is (false? (get-in result [:elle :valid?])))
      (is (contains? (set (get-in result [:elle :anomaly-types]))
                     :duplicate-elements))))

  (testing "a run without a successful read is not accepted"
    (let [result (check-in-temp-store (ops [[:append 1 1] [:append 1 1]]))]
      (is (= [:r] (get-in result [:meaningful-operations :missing-ok]))))))
